//! Where an export is written: a directory, or (in `s3`) a bucket. A name
//! is written whole or not at all and never replaces another, so a reader
//! that goes by the manifests never sees a part of a file, and two exports
//! that race for one manifest name cannot both win it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::fs::Fs;

/// Why a target call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetError {
    /// Something already has the name.
    Exists,
    /// The target failed; the text says how.
    Io(String),
}

impl std::fmt::Display for TargetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TargetError::Exists => f.write_str("the name is already taken"),
            TargetError::Io(why) => f.write_str(why),
        }
    }
}

/// Where an export's files go. Paths are relative and `/`-separated, such
/// as `tenant=<id>/date=2026-11-05/part-<id>.parquet`.
pub trait Target {
    /// Writes `bytes` as `path` if nothing has that name, durably once this
    /// returns Ok. On an error no file of that name is left with part of
    /// the bytes.
    fn put_new(&self, path: &str, bytes: &[u8]) -> Result<(), TargetError>;
    /// The names directly under the directory `dir` (ending in `/`),
    /// sorted; none when there is no such directory.
    fn list(&self, dir: &str) -> Result<Vec<String>, TargetError>;
    /// Removes `path`; one already gone is removed.
    fn remove(&self, path: &str) -> Result<(), TargetError>;
    /// Where the target is, as the command line names it.
    fn describe(&self) -> String;
}

/// A directory on this host, written through `Fs`: each file under a
/// temporary name, synced, renamed to its own name only if that is free,
/// and every directory it is in synced.
pub struct DirTarget {
    root: PathBuf,
    fs: Arc<dyn Fs>,
}

impl std::fmt::Debug for DirTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirTarget")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl DirTarget {
    /// The directory `root`, made if missing.
    pub fn new(root: &Path, fs: Arc<dyn Fs>) -> DirTarget {
        DirTarget {
            root: root.to_owned(),
            fs,
        }
    }

    /// Makes `dir` and syncs it and each directory above it up to the
    /// root's parent, so a new directory survives a crash.
    fn make_dir(&self, dir: &Path) -> Result<(), TargetError> {
        self.fs.create_dir_all(dir).map_err(io)?;
        let top = self.root.parent().unwrap_or(&self.root);
        let mut at = Some(dir);
        while let Some(d) = at {
            self.fs.sync_dir(d).map_err(io)?;
            if d == top || !d.starts_with(top) {
                break;
            }
            at = d.parent();
        }
        Ok(())
    }
}

fn io(e: std::io::Error) -> TargetError {
    TargetError::Io(e.to_string())
}

impl Target for DirTarget {
    fn put_new(&self, path: &str, bytes: &[u8]) -> Result<(), TargetError> {
        let to = self.root.join(path);
        let (Some(dir), Some(name)) = (to.parent(), to.file_name()) else {
            return Err(TargetError::Io(format!("{path} is not a file name")));
        };
        self.make_dir(dir)?;
        let mut temporary = name.to_owned();
        temporary.push(".tmp");
        let tmp = dir.join(temporary);
        let written = self
            .fs
            .write_new(&tmp, bytes)
            .and_then(|()| self.fs.sync_file(&tmp))
            .map_err(io)
            .and_then(|()| match self.fs.rename_new(&tmp, &to) {
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(TargetError::Exists),
                other => other.map_err(io),
            });
        if let Err(e) = written {
            // Best effort: a temporary name is never read as a file.
            let _ = self.fs.remove(&tmp);
            return Err(e);
        }
        self.fs.sync_dir(dir).map_err(io)
    }

    fn list(&self, dir: &str) -> Result<Vec<String>, TargetError> {
        match self.fs.list(&self.root.join(dir)) {
            Ok(names) => Ok(names
                .into_iter()
                .filter_map(|n| n.into_string().ok())
                .collect()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(io(e)),
        }
    }

    fn remove(&self, path: &str) -> Result<(), TargetError> {
        let at = self.root.join(path);
        match self.fs.remove(&at) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(io(e)),
            _ => {}
        }
        match at.parent() {
            Some(dir) => self.fs.sync_dir(dir).map_err(io),
            None => Ok(()),
        }
    }

    fn describe(&self) -> String {
        self.root.display().to_string()
    }
}

/// Where `--to` points: a directory, or an `s3://bucket/prefix` URL, whose
/// prefix is empty or ends in `/`. None for an `s3://` URL with no bucket
/// or with an empty part in its prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    Dir(PathBuf),
    S3 { bucket: String, prefix: String },
}

impl Place {
    /// The place `text` names.
    pub fn parse(text: &str) -> Option<Place> {
        let Some(rest) = text.strip_prefix("s3://") else {
            return (!text.is_empty()).then(|| Place::Dir(PathBuf::from(text)));
        };
        let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
        let parts: Vec<&str> = prefix.trim_end_matches('/').split('/').collect();
        if bucket.is_empty() || (!prefix.is_empty() && parts.iter().any(|p| p.is_empty())) {
            return None;
        }
        if prefix.ends_with("//") {
            return None;
        }
        let prefix = match prefix.trim_end_matches('/') {
            "" => String::new(),
            p => format!("{p}/"),
        };
        Some(Place::S3 {
            bucket: bucket.to_owned(),
            prefix,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_is_a_directory_or_a_bucket_and_prefix() {
        let s3 = |bucket: &str, prefix: &str| {
            Some(Place::S3 {
                bucket: bucket.to_owned(),
                prefix: prefix.to_owned(),
            })
        };
        for (text, place) in [
            ("/srv/export", Some(Place::Dir("/srv/export".into()))),
            ("out", Some(Place::Dir("out".into()))),
            ("s3://reports", s3("reports", "")),
            ("s3://reports/", s3("reports", "")),
            ("s3://reports/vcon", s3("reports", "vcon/")),
            ("s3://reports/vcon/2026/", s3("reports", "vcon/2026/")),
            ("s3://", None),
            ("s3:///vcon", None),
            ("s3://reports//vcon", None),
            ("s3://reports/vcon//", None),
            ("", None),
        ] {
            assert_eq!(Place::parse(text), place, "{text:?}");
        }
    }
    use crate::fs::{CrashFs, OsFs};

    #[test]
    fn a_name_is_written_once_and_never_replaced() {
        let dir = tempfile::tempdir().expect("temp dir");
        let target = DirTarget::new(&dir.path().join("out"), Arc::new(OsFs));
        target.put_new("a/b/one", b"first").expect("writes");
        assert_eq!(
            target.put_new("a/b/one", b"second"),
            Err(TargetError::Exists),
            "a taken name is refused"
        );
        assert_eq!(
            std::fs::read(dir.path().join("out/a/b/one")).expect("reads"),
            b"first",
            "the first stays"
        );
        assert_eq!(
            target.list("a/b/").expect("lists"),
            ["one"],
            "no temporary name is left"
        );
        assert!(target.list("none/").expect("lists").is_empty(), "no dir");
        target.remove("a/b/one").expect("removes");
        target.remove("a/b/one").expect("removes again");
        assert!(target.list("a/b/").expect("lists").is_empty(), "gone");
    }

    /// Once `put_new` returns, a crash keeps the file, its directories
    /// and its bytes.
    #[test]
    fn a_written_file_survives_a_crash() {
        let dir = tempfile::tempdir().expect("temp dir");
        let fs = Arc::new(CrashFs::new(dir.path()));
        let target = DirTarget::new(&dir.path().join("out"), fs.clone());
        target.put_new("t/d/f.parquet", b"whole").expect("writes");
        fs.crash();
        assert_eq!(
            fs.read(&dir.path().join("out/t/d/f.parquet"))
                .expect("kept"),
            b"whole",
            "the file and its bytes survive the crash"
        );
    }
}
