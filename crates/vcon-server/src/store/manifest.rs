//! The numbered manifests of Parquet storage, for readers outside the
//! server. Each commit that changes which files hold which rows (a pack, a
//! purge's rewrite, a compaction) takes the next manifest number in its
//! own transaction. After the commit, the manifest is written under that
//! number, with `If-None-Match: *`, from one snapshot of the catalog: the
//! files the catalog names, with their rows and lengths, and the versions
//! that are live (current, of a uuid not deleted). The catalog stays the
//! only commit point; a manifest is what a reader outside reads from it.
//!
//! A reader lists `manifest/`, takes the highest number (the names sort in
//! order) and reads the files it names. A file a compaction stops naming
//! stays for the reader grace, so a reader on an older manifest can finish;
//! a file a purge rewrote is deleted at once, and every older manifest with
//! it, since those name it.

use std::collections::BTreeSet;
use std::time::Duration;

use time::format_description::well_known::Rfc3339;

use super::{Store, StoreError, commit, io, read_failed};
use crate::clock::nanos;

/// The format written in each manifest.
pub(crate) const FORMAT: &str = "vcon-server.parquet-storage/1";

/// The name of manifest `seq` under the tenant's directory.
pub(crate) fn name(seq: i64) -> String {
    format!("manifest/{seq:020}.json")
}

impl Store {
    /// Writes every manifest numbered and not yet written, oldest first,
    /// and marks it written. One already in the storage under its name (a
    /// write cut off before its mark) is marked as it is.
    pub(crate) fn publish(&self) -> Result<(), StoreError> {
        let Some(packs) = self.packs.as_deref() else {
            return Ok(());
        };
        // Bound first, so the snapshot (which holds the catalog) ends here,
        // not at the end of the loop, whose body takes the catalog again.
        let unpublished = self.catalog.snapshot()?.unpublished_manifests()?;
        for seq in unpublished {
            let (files, live) = {
                let mut tx = self.catalog.snapshot()?;
                (tx.named_files()?, tx.live_versions()?)
            };
            let now = self.clock.now();
            let listed: Vec<serde_json::Value> = files
                .iter()
                .map(|(file, info, rows)| {
                    serde_json::json!({
                        "path": crate::backend::pack::name(&info.date, *file),
                        "rows": rows,
                        "bytes": info.bytes,
                    })
                })
                .collect();
            let manifest = serde_json::json!({
                "format": FORMAT,
                "tenant": self.tenant.to_string(),
                "seq": seq,
                "written_at": now.format(&Rfc3339).map_err(io)?,
                "files": listed,
                "rows": files.iter().map(|(_, _, rows)| rows).sum::<i64>(),
                "bytes": files.iter().map(|(_, info, _)| info.bytes).sum::<u64>(),
                "live": live.iter().map(ToString::to_string).collect::<Vec<_>>(),
            });
            let text = format!("{manifest:#}\n");
            // A name already taken holds this number's manifest, written by
            // a run cut off before its mark: it stands.
            packs
                .put_new(&name(seq), text.as_bytes())
                .map_err(read_failed)?;
            let mut tx = self.catalog.immediate()?;
            tx.mark_published(seq, nanos(now))?;
            commit(tx)?;
        }
        Ok(())
    }

    /// Numbers a new manifest when the newest one written no longer says
    /// what the catalog does: the files it names, or the live versions (a
    /// delete or a restore changes them and no file). A Store with pack
    /// files and no manifest gets its first.
    pub(crate) fn refresh_manifest(&self) -> Result<(), StoreError> {
        let Some(packs) = self.packs.as_deref() else {
            return Ok(());
        };
        let (unpublished, written, files, live) = {
            let mut tx = self.catalog.snapshot()?;
            (
                tx.unpublished_manifests()?,
                tx.published_manifests()?,
                tx.named_files()?,
                tx.live_versions()?,
            )
        };
        if !unpublished.is_empty() {
            // `publish` writes it from the catalog as it is then.
            return Ok(());
        }
        let paths: BTreeSet<String> = files
            .iter()
            .map(|(file, info, _)| crate::backend::pack::name(&info.date, *file))
            .collect();
        let live: BTreeSet<String> = live.iter().map(ToString::to_string).collect();
        let current = match written.last() {
            None => paths.is_empty(),
            Some((seq, _)) => {
                let name = name(*seq);
                let size = packs.size(&name).map_err(read_failed)?;
                let text = match size {
                    Some(size) => packs.read(&name, 0, size).map_err(read_failed)?,
                    None => Vec::new(),
                };
                let said: serde_json::Value =
                    serde_json::from_slice(&text).unwrap_or(serde_json::Value::Null);
                let strings = |key: &str, field: Option<&str>| -> BTreeSet<String> {
                    said[key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| match field {
                            Some(f) => v[f].as_str().map(str::to_owned),
                            None => v.as_str().map(str::to_owned),
                        })
                        .collect()
                };
                strings("files", Some("path")) == paths && strings("live", None) == live
            }
        };
        if !current {
            let mut tx = self.catalog.immediate()?;
            tx.allocate_manifest()?;
            commit(tx)?;
        }
        Ok(())
    }

    /// Removes the manifests a reader no longer needs: every one but the
    /// newest whose next one was written at least `grace` ago; with
    /// `grace` None, every one but the newest.
    pub(crate) fn prune_manifests(&self, grace: Option<Duration>) -> Result<(), StoreError> {
        let Some(packs) = self.packs.as_deref() else {
            return Ok(());
        };
        let written = self.catalog.snapshot()?.published_manifests()?;
        let now = nanos(self.clock.now());
        let grace = grace.map(|g| i64::try_from(g.as_nanos()).unwrap_or(i64::MAX));
        for pair in written.windows(2) {
            let ((seq, _), (_, next_at)) = (pair[0], pair[1]);
            let past = grace.is_none_or(|g| now.saturating_sub(next_at) >= g);
            if !past {
                continue;
            }
            packs.remove(&name(seq)).map_err(read_failed)?;
            let mut tx = self.catalog.immediate()?;
            tx.delete_manifest(seq)?;
            commit(tx)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use serde_json::Value;
    use time::Duration as Span;

    use super::*;
    use crate::backend::Packs;
    use crate::clock::Clock;
    use crate::retention::Retention;
    use crate::store::Created;
    use crate::store::packed::tests::DirPacks;
    use crate::store::packer::PackPlan;
    use crate::store::packer::tests::{Kept, with_packs};
    use crate::store::testing::vcon;
    use crate::store::tests::{Fixture, fixture};

    /// Packing without merging, so each run writes the manifests these
    /// tests count; compaction's are its own tests'.
    fn plan() -> PackPlan {
        PackPlan {
            merge_below: 0,
            ..PackPlan::with_limit(1 << 20)
        }
    }

    fn created(f: &Fixture, n: u8, subject: &str) -> Kept {
        let (found, body) = vcon(
            &format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc{n:02x}"),
            subject,
        );
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        Kept {
            uuid: found.uuid,
            version,
            body,
        }
    }

    /// Each manifest in the storage, by number, parsed.
    fn manifests(packs: &DirPacks) -> Vec<(i64, Value)> {
        let dir = packs.path("manifest");
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .map(|d| {
                d.map(|e| {
                    e.expect("an entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
            .into_iter()
            .map(|n| {
                let seq: i64 = n.trim_end_matches(".json").parse().expect("a number");
                assert_eq!(name(seq), format!("manifest/{n}"), "20 digits");
                let text = std::fs::read_to_string(dir.join(&n)).expect("read");
                (seq, serde_json::from_str(&text).expect("JSON"))
            })
            .collect()
    }

    /// The paths a manifest names, and its live versions.
    fn named(m: &Value) -> (BTreeSet<String>, BTreeSet<String>) {
        let files = m["files"]
            .as_array()
            .expect("files")
            .iter()
            .map(|f| f["path"].as_str().expect("a path").to_owned())
            .collect();
        let live = m["live"]
            .as_array()
            .expect("live")
            .iter()
            .map(|v| v.as_str().expect("an id").to_owned())
            .collect();
        (files, live)
    }

    fn files(packs: &DirPacks) -> BTreeSet<String> {
        packs.list().expect("listed").into_iter().collect()
    }

    /// Each change of which files hold which rows writes the next manifest:
    /// it names exactly the files the storage holds, with their rows and
    /// lengths, and the live versions; a purge's rewrite writes one too.
    #[test]
    fn each_change_of_files_writes_the_next_manifest() {
        let mut f = fixture();
        let a = created(&f, 1, "a");
        let packs = with_packs(&mut f);
        f.store.pack(&plan()).expect("packs");
        let first = manifests(&packs);
        assert_eq!(first.len(), 1, "one manifest");
        let (seq, m) = &first[0];
        assert_eq!(*seq, 1, "numbered from 1");
        assert_eq!(m["format"], FORMAT, "its format");
        assert_eq!(m["seq"], 1, "its own number");
        assert_eq!(m["tenant"], f.tenant().to_string(), "its tenant");
        assert_eq!(named(m).0, files(&packs), "the files the storage holds");
        assert_eq!(m["files"][0]["rows"], 1, "the file's rows");
        let size = packs.size(m["files"][0]["path"].as_str().expect("a path"));
        assert_eq!(
            Some(m["files"][0]["bytes"].as_u64().expect("a length")),
            size.expect("a length"),
            "the file's length"
        );
        assert_eq!(named(m).1, BTreeSet::from([a.version.to_string()]), "live");
        let (found, body) = vcon(&format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc{:02x}", 1), "b");
        f.store
            .replace(&f.caller, a.version, &found, &body)
            .expect("replaced");
        let b = created(&f, 2, "c");
        f.store.pack(&plan()).expect("packs");
        let second = manifests(&packs);
        let (seq, m) = second.last().expect("a manifest");
        assert_eq!(*seq, 2, "the next number");
        assert_eq!(named(m).0, files(&packs), "both files");
        let current = f.store.get(&f.caller, a.uuid).expect("reads").version;
        assert_eq!(
            named(m).1,
            BTreeSet::from([current.to_string(), b.version.to_string()]),
            "the live versions: not the replaced one"
        );
        f.store
            .delete(&f.caller, b.uuid, Retention::Days(0))
            .expect("deleted and purged");
        let third = manifests(&packs);
        let (seq, m) = third.last().expect("a manifest");
        assert_eq!(*seq, 3, "a purge's rewrite writes the next one");
        assert_eq!(named(m).0, files(&packs), "the files after the rewrite");
        assert_eq!(named(m).1, BTreeSet::from([current.to_string()]), "live");
        let unpublished: i64 = f
            .store
            .conn_for_test()
            .query_row(
                "SELECT count(*) FROM manifest WHERE published_at IS NULL",
                [],
                |r| r.get(0),
            )
            .expect("counted");
        assert_eq!(unpublished, 0, "every manifest is marked written");
    }

    /// A purge removes every manifest older than the one its rewrite
    /// wrote: those name the file it deleted.
    #[test]
    fn a_purge_removes_every_older_manifest() {
        let mut f = fixture();
        let a = created(&f, 1, "a");
        let packs = with_packs(&mut f);
        f.store.pack(&plan()).expect("packs");
        created(&f, 2, "b");
        f.store.pack(&plan()).expect("packs");
        assert_eq!(manifests(&packs).len(), 2, "two manifests");
        f.store
            .delete(&f.caller, a.uuid, Retention::Days(0))
            .expect("deleted and purged");
        let left = manifests(&packs);
        assert_eq!(left.len(), 1, "only the newest: {left:?}");
        assert_eq!(left[0].0, 3, "the purge's own");
        let rows: i64 = f
            .store
            .conn_for_test()
            .query_row("SELECT count(*) FROM manifest", [], |r| r.get(0))
            .expect("counted");
        assert_eq!(rows, 1, "and the catalog's rows with them");
    }

    /// A manifest numbered but not written, when the write fails, is
    /// written at the next start; one written but not marked is marked as
    /// it is.
    #[test]
    fn a_manifest_left_unwritten_is_written_at_the_next_start() {
        let mut f = fixture();
        created(&f, 1, "a");
        let packs = with_packs(&mut f);
        let plan = plan();
        let planned = f.store.plan_files(&plan).expect("planned");
        let written = f
            .store
            .write_file(&planned[0], &plan)
            .expect("written")
            .expect("a file");
        assert_eq!(f.store.record_file(&written), Ok(true), "recorded");
        assert!(manifests(&packs).is_empty(), "not written yet");
        f.reopen();
        assert_eq!(manifests(&packs).len(), 1, "written at the start");
        // A second manifest, written but not marked, is only marked.
        created(&f, 2, "b");
        let planned = f.store.plan_files(&plan).expect("planned");
        let written = f
            .store
            .write_file(&planned[0], &plan)
            .expect("written")
            .expect("a file");
        assert_eq!(f.store.record_file(&written), Ok(true), "recorded");
        let text = br#"{"written":"before a crash"}"#;
        assert!(packs.put_new(&name(2), text).expect("put"), "a new name");
        f.store.publish().expect("published");
        assert_eq!(
            std::fs::read(packs.path(&name(2))).expect("read"),
            text,
            "the manifest already there is kept"
        );
        let unpublished: i64 = f
            .store
            .conn_for_test()
            .query_row(
                "SELECT count(*) FROM manifest WHERE published_at IS NULL",
                [],
                |r| r.get(0),
            )
            .expect("counted");
        assert_eq!(unpublished, 0, "every manifest is marked written");
    }

    /// A manifest's write that fails leaves it numbered and the pack run
    /// failed; the versions are packed and read back.
    #[test]
    fn a_manifest_write_that_fails_fails_the_run_and_packs_anyway() {
        let mut f = fixture();
        let a = created(&f, 1, "a");
        let packs = with_packs(&mut f);
        packs.fail_manifests.store(true, Ordering::SeqCst);
        assert_eq!(f.store.pack(&plan()), Err(StoreError::Io), "the run fails");
        assert_eq!(
            f.store.get(&f.caller, a.uuid).map(|s| s.body),
            Ok(a.body.clone()),
            "the version reads back"
        );
        packs.fail_manifests.store(false, Ordering::SeqCst);
        f.store.pack(&plan()).expect("the next run");
        assert_eq!(manifests(&packs).len(), 1, "written by the next run");
    }

    /// `publish` with manifests numbered and not written finishes. Run on
    /// a thread of its own with a deadline: a call that takes the catalog
    /// while it holds it (a snapshot kept alive across the loop that takes
    /// the catalog again) would wait forever, and fails here by name
    /// instead of stopping the whole test binary.
    #[test]
    fn publishing_numbered_manifests_ends_without_waiting_on_itself() {
        let mut f = fixture();
        created(&f, 1, "a");
        created(&f, 2, "b");
        let packs = with_packs(&mut f);
        let plan = plan();
        for planned in f.store.plan_files(&plan).expect("planned") {
            let written = f
                .store
                .write_file(&planned, &plan)
                .expect("written")
                .expect("a file");
            assert_eq!(f.store.record_file(&written), Ok(true), "recorded");
        }
        let mut store = f.open_beside_store();
        store.packs = Some(packs.clone());
        let store = std::sync::Arc::new(store);
        let (done, finished) = std::sync::mpsc::channel();
        let publisher = std::thread::spawn(move || {
            let _ = done.send(store.publish());
        });
        match finished.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(published) => assert_eq!(published, Ok(()), "published"),
            Err(_) => {
                panic!("publish did not end within 5 seconds: it waits on the catalog it holds")
            }
        }
        // The thread still holds the Store and closes its catalog as it
        // ends. Wait for that: a test process that exits while SQLCipher is
        // still in use on another thread can crash (SIGSEGV) after the test
        // passed, as it did under coverage in CI.
        publisher.join().expect("the publishing thread ends");
        assert!(!manifests(&packs).is_empty(), "written");
    }

    /// A delete or a restore changes which versions are live, and no file:
    /// the next pack run writes a manifest with the live versions as they
    /// are; a run with nothing changed writes none.
    #[test]
    fn a_pack_run_writes_a_manifest_when_the_live_versions_change() {
        let mut f = fixture();
        let a = created(&f, 1, "a");
        let b = created(&f, 2, "b");
        let packs = with_packs(&mut f);
        f.store.pack(&plan()).expect("packs");
        assert_eq!(manifests(&packs).len(), 1, "one manifest");
        f.store.pack(&plan()).expect("packs");
        assert_eq!(manifests(&packs).len(), 1, "nothing changed: no manifest");
        f.store
            .delete(&f.caller, b.uuid, Retention::Years(7))
            .expect("deleted");
        f.store.pack(&plan()).expect("packs");
        let after_delete = manifests(&packs);
        let (seq, m) = after_delete.last().expect("a manifest");
        assert_eq!(*seq, 2, "a new manifest");
        assert_eq!(
            named(m).1,
            BTreeSet::from([a.version.to_string()]),
            "b is not live"
        );
        assert_eq!(named(m).0, files(&packs), "the same files");
        f.store.restore(&f.caller, b.uuid).expect("restored");
        f.store.pack(&plan()).expect("packs");
        let (seq, m) = manifests(&packs).last().cloned().expect("a manifest");
        assert_eq!(seq, 3, "a new manifest");
        assert_eq!(
            named(&m).1,
            BTreeSet::from([a.version.to_string(), b.version.to_string()]),
            "b is live again"
        );
    }

    /// Older manifests stay for the reader grace after a newer one is
    /// written, then a pack run removes them; the newest always stays.
    #[test]
    fn older_manifests_go_after_the_reader_grace() {
        let mut f = fixture();
        let packs = with_packs(&mut f);
        let grace = std::time::Duration::from_secs(900);
        let plan = PackPlan { grace, ..plan() };
        created(&f, 1, "a");
        f.store.pack(&plan).expect("packs");
        created(&f, 2, "b");
        f.store.pack(&plan).expect("packs");
        assert_eq!(manifests(&packs).len(), 2, "within the grace, both stay");
        f.clock.set(f.clock.now() + Span::seconds(899));
        f.store.pack(&plan).expect("packs");
        assert_eq!(manifests(&packs).len(), 2, "a second before the grace ends");
        f.clock.set(f.clock.now() + Span::seconds(1));
        f.store.pack(&plan).expect("packs");
        let left = manifests(&packs);
        assert_eq!(left.len(), 1, "the older one goes: {left:?}");
        assert_eq!(left[0].0, 2, "the newest stays");
        f.clock.set(f.clock.now() + Span::days(30));
        f.store.pack(&plan).expect("packs");
        assert_eq!(manifests(&packs).len(), 1, "the newest stays for good");
    }
}
