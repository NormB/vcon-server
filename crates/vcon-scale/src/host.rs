//! Facts about the host and the server process, read from text the caller
//! supplies (lscpu, /proc), so each parser is tested on fixed input.

use std::path::Path;

/// The value of `name:` in `lscpu`'s output, trimmed; `None` when missing,
/// empty or "-".
#[must_use]
pub fn lscpu_field(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        let value = value.trim();
        (key.trim() == name && !value.is_empty() && value != "-").then(|| value.to_owned())
    })
}

/// The machine's name: the device tree's model (`/proc/device-tree/model`,
/// NUL-terminated) when there is one, else lscpu's "Model name".
#[must_use]
pub fn machine(device_tree_model: Option<&str>, lscpu: &str) -> String {
    device_tree_model
        .map(|model| model.trim_end_matches('\0').trim())
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
        .or_else(|| lscpu_field(lscpu, "Model name"))
        .unwrap_or_else(|| "unknown machine".to_owned())
}

/// The kibibytes of `name:` in `/proc/meminfo` or `/proc/<pid>/status`
/// (lines `Name:   123 kB`).
#[must_use]
pub fn kib_field(text: &str, name: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key != name {
            return None;
        }
        value.split_whitespace().next()?.parse().ok()
    })
}

/// The nanoseconds on CPU of every thread of a process, from its
/// `/proc/<pid>/stat` (utime plus stime, fields 14 and 15, in ticks) and
/// the ticks a second (`getconf CLK_TCK`). `/proc/<pid>/schedstat` would
/// count the main thread alone.
#[must_use]
pub fn stat_cpu_ns(stat: &str, ticks_per_second: u64) -> Option<u64> {
    if ticks_per_second == 0 {
        return None;
    }
    // The command name, field 2, sits in parentheses and may hold either;
    // the fields after its last ')' start at field 3.
    let (_, rest) = stat.rsplit_once(')')?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let utime: u64 = fields.get(14 - 3)?.parse().ok()?;
    let stime: u64 = fields.get(15 - 3)?.parse().ok()?;
    let ticks = u128::from(utime) + u128::from(stime);
    u64::try_from(ticks * 1_000_000_000 / u128::from(ticks_per_second)).ok()
}

/// The bytes of every regular file under `root`, symbolic links not
/// followed. The server runs while the tree is read, so a file or a
/// directory below `root` that is removed between its listing and its
/// reading counts nothing.
///
/// # Errors
///
/// `root` that cannot be read, and the first other error reading a
/// directory or a file's metadata.
pub fn tree_bytes(root: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    let mut pending = vec![root.to_path_buf()];
    std::fs::read_dir(root)?;
    while let Some(dir) = pending.pop() {
        for path in subtree_entries(&dir)? {
            let meta = std::fs::symlink_metadata(&path);
            if matches!(&meta, Ok(m) if m.is_dir()) {
                pending.push(path);
            } else {
                total = total.saturating_add(file_bytes(meta)?);
            }
        }
    }
    Ok(total)
}

/// The bytes of `entry` when it is a regular file, else 0; a file removed
/// since it was listed is 0.
///
/// # Errors
///
/// An error reading its metadata other than its absence.
pub fn entry_bytes(entry: &std::fs::DirEntry) -> std::io::Result<u64> {
    file_bytes(entry.metadata())
}

/// The bytes a listed entry counts, from its metadata (symbolic links not
/// followed): a regular file's length, 0 for anything else and for an
/// entry removed since it was listed.
fn file_bytes(meta: std::io::Result<std::fs::Metadata>) -> std::io::Result<u64> {
    match meta {
        Ok(meta) if meta.is_file() => Ok(meta.len()),
        Ok(_) => Ok(0),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e),
    }
}

/// The paths listed in `dir`; none when `dir` was removed.
///
/// # Errors
///
/// An error listing it other than its absence.
pub fn subtree_entries(dir: &Path) -> std::io::Result<Vec<std::path::PathBuf>> {
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut paths = Vec::new();
    for entry in listing {
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(paths)
}
