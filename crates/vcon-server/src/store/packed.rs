//! Reading a version's sealed blob where it is kept: its own object, or its
//! row in a pack file.
//!
//! A packed row is read with the file's footer, read once and kept (a file
//! never changes, so a kept footer never goes stale), and one ranged read
//! of the row's own page. Every way a pack read can go wrong without the
//! storage failing (no pack storage, a file shorter or longer than its
//! row says, an end that is not a pack's, a place the file does not hold,
//! a page that is not one value) is damage, as a missing or altered object
//! is; the AES-GCM open that follows catches a page of another row.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::Store;
use crate::backend::pack::{self, Footer, Parsed};
use crate::backend::{ObjectError, Packed, Packs};
use crate::ids::{FileId, VersionId};

/// How many files' footers a Store keeps.
pub(super) const FOOTERS: usize = 64;

/// Why a sealed blob was not read.
#[derive(Debug)]
pub(super) enum Unread {
    /// The storage failed, or holds no object or file of the name.
    Object(ObjectError),
    /// What the storage holds is not what the row says.
    Damaged,
}

impl Unread {
    /// How a read that needed the blob answers: an object or file the
    /// storage does not hold is damage, as is one that is not what its row
    /// says; any other failure is the storage's.
    pub(super) fn into_store(self) -> super::StoreError {
        match self {
            Unread::Object(e) => super::read_failed(e),
            Unread::Damaged => super::StoreError::Damaged,
        }
    }
}

/// The footers of the pack files read, by file id; at most `cap`, after
/// which they are all dropped and read again as needed.
#[derive(Debug)]
pub(super) struct Footers {
    cap: usize,
    files: Mutex<HashMap<FileId, Arc<Footer>>>,
}

impl Footers {
    pub(super) fn new(cap: usize) -> Footers {
        Footers {
            cap,
            files: Mutex::new(HashMap::new()),
        }
    }
}

impl Store {
    /// The sealed blob of `version`: its object, or, when `place` says it
    /// is packed, its row in its pack file.
    pub(super) fn sealed_blob(
        &self,
        version: VersionId,
        place: Option<&Packed>,
    ) -> Result<Vec<u8>, Unread> {
        let Some(packed) = place else {
            return self.objects.get(version).map_err(Unread::Object);
        };
        let packs = self.packs.as_deref().ok_or(Unread::Damaged)?;
        let name = pack::name(&packed.date, packed.file);
        let footer = self.footer(packs, packed, &name)?;
        let page = footer.page(packed.place).ok_or(Unread::Damaged)?;
        let bytes = read(packs, &name, page.start, page.end - page.start)?;
        footer.sealed(packed.place, bytes).ok_or(Unread::Damaged)
    }

    /// The footer of `packed`'s file: kept from an earlier read, or read
    /// from the file's end, at the length its row says, and kept.
    fn footer(
        &self,
        packs: &dyn Packs,
        packed: &Packed,
        name: &str,
    ) -> Result<Arc<Footer>, Unread> {
        if let Some(footer) = self.footers.lock().get(&packed.file) {
            return Ok(footer.clone());
        }
        let size = packed.bytes;
        let mut want = pack::TAIL.min(size);
        // The footer's length, then its offset index's. Each read must be
        // longer than the last, so the loop ends whatever the parser asks;
        // parquet asks for more only when it has less, so no test reaches
        // the refusal.
        let footer = loop {
            let tail = read(packs, name, size - want, want)?;
            match Footer::parse(tail, size).ok_or(Unread::Damaged)? {
                Parsed::Footer(footer) => break Arc::new(footer),
                Parsed::More(more) if more > want && more <= size => want = more,
                Parsed::More(_) => return Err(Unread::Damaged),
            }
        };
        let mut files = self.footers.lock();
        if files.len() >= self.footers.cap {
            files.clear();
        }
        files.insert(packed.file, footer.clone());
        Ok(footer)
    }
}

impl Footers {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<FileId, Arc<Footer>>> {
        self.files
            .lock()
            .expect("no thread panicked holding the footers")
    }
}

/// `len` bytes of `name` from `start`. Fewer, from a file shorter than its
/// row says, are not refused here: a short page fails the page reader's
/// bounds, and a short tail fails the footer's parse or gives pages that
/// fail their open, so each is damage where it is read.
fn read(packs: &dyn Packs, name: &str, start: u64, len: u64) -> Result<Vec<u8>, Unread> {
    packs.read(name, start, len).map_err(Unread::Object)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::backend::pack::Place;
    use crate::export::Metadata;
    use crate::export::columns::{About, File, Row, Sizes};
    use crate::store::testing::vcon;
    use crate::store::tests::{Fixture, fixture};
    use crate::store::{Created, StoreError};

    /// Pack files in a directory, under the names the bucket would give
    /// them, with every read recorded.
    #[derive(Debug)]
    pub(crate) struct DirPacks {
        pub(crate) root: tempfile::TempDir,
        pub(crate) reads: Mutex<Vec<(String, u64, u64)>>,
    }

    impl DirPacks {
        pub(crate) fn new() -> Arc<DirPacks> {
            Arc::new(DirPacks {
                root: tempfile::tempdir().expect("temp dir"),
                reads: Mutex::new(Vec::new()),
            })
        }

        pub(crate) fn path(&self, name: &str) -> PathBuf {
            self.root.path().join(name)
        }

        /// The reads made since the last call, and none kept.
        pub(crate) fn taken(&self) -> Vec<(String, u64, u64)> {
            std::mem::take(&mut *self.reads.lock().expect("reads"))
        }
    }

    impl Packs for DirPacks {
        fn read(&self, name: &str, start: u64, len: u64) -> Result<Vec<u8>, ObjectError> {
            self.reads
                .lock()
                .expect("reads")
                .push((name.to_owned(), start, len));
            let bytes = match std::fs::read(self.path(name)) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(ObjectError::Missing);
                }
                Err(e) => return Err(ObjectError::Io(e)),
            };
            let from = usize::try_from(start).expect("fits").min(bytes.len());
            let to = from
                .saturating_add(usize::try_from(len).expect("fits"))
                .min(bytes.len());
            Ok(bytes[from..to].to_vec())
        }

        fn size(&self, name: &str) -> Result<Option<u64>, ObjectError> {
            match std::fs::metadata(self.path(name)) {
                Ok(m) => Ok(Some(m.len())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(ObjectError::Io(e)),
            }
        }
    }

    /// The date every test version is stored on: the fixture's clock.
    pub(crate) const DATE: &str = "2026-10-23";

    /// Packs `versions` of `store` into one new file, which `put` stores
    /// under its name, as the packer does: the file is written, then its
    /// row and each version's place are committed, then the versions'
    /// objects are removed. Row groups hold about `group` bytes. Returns
    /// the file's id and bytes, and each version's place.
    pub(crate) fn pack_with(
        store: &Store,
        put: &dyn Fn(&str, &[u8]),
        versions: &[VersionId],
        group: usize,
    ) -> (FileId, Vec<u8>, Vec<Place>) {
        let sizes = Sizes {
            row_group: group,
            file: usize::MAX,
        };
        let mut file = File::new(Metadata::None, sizes).expect("a new file");
        let mut places = Vec::new();
        for &version in versions {
            let stored_at: i64 = store
                .conn_for_test()
                .query_row(
                    "SELECT stored_at FROM version WHERE version = ?1",
                    [version.0],
                    |r| r.get(0),
                )
                .expect("a version row");
            let sealed = store.objects.get(version).expect("its object");
            places.push(
                file.push(Row {
                    version,
                    stored_at,
                    sealed,
                    about: About::default(),
                })
                .expect("adds a row"),
            );
        }
        let bytes = file.finish().expect("the file");
        let id = FileId::generate();
        put(&pack::name(DATE, id), &bytes);
        {
            let conn = store.conn_for_test();
            let tx = conn.unchecked_transaction().expect("begin");
            tx.execute(
                "INSERT INTO parquet_file (file_id, date, rows, live_rows, bytes, created_at)
                 VALUES (?1, ?2, ?3, ?3, ?4, 0)",
                rusqlite::params![id.0, DATE, versions.len() as i64, bytes.len() as i64],
            )
            .expect("the file's row");
            for (version, place) in versions.iter().zip(&places) {
                tx.execute(
                    "UPDATE version SET place_file = ?1, place_group = ?2, place_row = ?3
                     WHERE version = ?4",
                    rusqlite::params![id.0, place.group, place.row, version.0],
                )
                .expect("the place");
            }
            tx.commit().expect("commits");
        }
        for &version in versions {
            store.objects.remove(version).expect("removed");
        }
        (id, bytes, places)
    }

    /// `pack_with`, into `packs`.
    pub(crate) fn pack(
        store: &Store,
        packs: &DirPacks,
        versions: &[VersionId],
        group: usize,
    ) -> (FileId, Vec<u8>, Vec<Place>) {
        let put = |name: &str, bytes: &[u8]| {
            let path = packs.path(name);
            std::fs::create_dir_all(path.parent().expect("a directory")).expect("made");
            std::fs::write(&path, bytes).expect("written");
        };
        pack_with(store, &put, versions, group)
    }

    const A: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
    const B: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc47";
    const C: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc48";

    /// A stored version: its uuid, version id, body and object.
    struct Kept {
        uuid: uuid::Uuid,
        version: VersionId,
        body: Vec<u8>,
        sealed: Vec<u8>,
    }

    fn created(f: &Fixture, uuid: &str, subject: &str) -> Kept {
        let (found, body) = vcon(uuid, subject);
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        let sealed = f.store.objects.get(version).expect("its object");
        Kept {
            uuid: found.uuid,
            version,
            body,
            sealed,
        }
    }

    /// A, replaced once (its first version hidden), B and C, each with a
    /// subject long enough that a few fill a row group.
    fn stored(f: &Fixture) -> Vec<Kept> {
        let a1 = created(f, A, &"first ".repeat(20));
        let (found, body) = vcon(A, &"second ".repeat(20));
        let a2 = f
            .store
            .replace(&f.caller, a1.version, &found, &body)
            .expect("replaced");
        let a2 = Kept {
            uuid: found.uuid,
            version: a2,
            sealed: f.store.objects.get(a2).expect("its object"),
            body,
        };
        let b = created(f, B, &"bee ".repeat(30));
        let c = created(f, C, &"sea ".repeat(40));
        vec![a1, a2, b, c]
    }

    fn with_packs(f: &mut Fixture, packs: &Arc<DirPacks>) {
        f.store.packs = Some(packs.clone());
    }

    /// Every read returns a packed version's body, byte for byte, as it did
    /// from the object: `get` the current one, `legal_get` a hidden one,
    /// the export its sealed blob; `check` finds nothing missing, and a
    /// restart's recovery keeps every version.
    #[test]
    fn every_read_of_a_packed_version_returns_what_its_object_held() {
        let mut f = fixture();
        let kept = stored(&f);
        let packs = DirPacks::new();
        with_packs(&mut f, &packs);
        let versions: Vec<VersionId> = kept.iter().map(|k| k.version).collect();
        let (_, _, places) = pack(&f.store, &packs, &versions, 400);
        assert!(
            places.iter().any(|p| p.group > 0),
            "more than one row group: {places:?}"
        );
        assert_eq!(
            f.store.objects.list().expect("listed"),
            Vec::<String>::new(),
            "no object is left"
        );
        for round in ["packed", "after a restart"] {
            for k in &kept[1..] {
                let got = f.store.get(&f.caller, k.uuid).expect("reads");
                assert_eq!(got.version, k.version, "{round}: the current version");
                assert_eq!(got.body, k.body, "{round}: the body, byte for byte");
            }
            let hidden = f
                .store
                .legal_get(&f.caller, kept[0].uuid, Some(kept[0].version))
                .expect("a legal read");
            assert_eq!(
                hidden.stored.body, kept[0].body,
                "{round}: a hidden version"
            );
            let listed = f.store.kept_versions(None, None).expect("listed");
            assert_eq!(listed.len(), kept.len(), "{round}: every version is kept");
            for l in &listed {
                let k = kept
                    .iter()
                    .find(|k| k.version == l.version)
                    .expect("a stored version");
                let sealed = f.store.sealed_of(l).expect("reads").expect("kept");
                assert_eq!(sealed, k.sealed, "{round}: the export's sealed blob");
            }
            assert_eq!(
                f.store.check().expect("checked"),
                Vec::<VersionId>::new(),
                "{round}: nothing is missing"
            );
            f.reopen();
            with_packs(&mut f, &packs);
        }
    }

    /// The SQL that gives every packed version the same place: the place of
    /// the version stored first.
    fn place_of_first(f: &Fixture) {
        f.store
            .conn_for_test()
            .execute_batch(
                "UPDATE version SET (place_group, place_row) =
                     (SELECT place_group, place_row FROM version
                      ORDER BY stored_at, rowid LIMIT 1)",
            )
            .expect("changed");
    }

    /// A packed version read where its row does not say, or from a file
    /// that is not what its row says, is damage, never another version's
    /// body: another row's place, a place the file does not hold, a file
    /// with no row, a file gone or cut short, a file whose length the row
    /// misstates, and a Store with no pack storage. (A file grown at its
    /// end still reads: its footer is read at the length the row says, and
    /// every value is checked by its open; `check` finds such a file.)
    #[test]
    fn a_packed_version_read_from_a_wrong_place_is_damage_never_another_body() {
        type Break = fn(&Fixture, &DirPacks, &str, &[u8]);
        let breaks: [(&str, Break); 7] = [
            ("another row's place", |f, _, _, _| place_of_first(f)),
            ("a row group the file does not hold", |f, _, _, _| {
                f.store
                    .conn_for_test()
                    .execute_batch("UPDATE version SET place_group = place_group + 7")
                    .expect("changed");
            }),
            ("a row its group does not hold", |f, _, _, _| {
                f.store
                    .conn_for_test()
                    .execute_batch("UPDATE version SET place_row = place_row + 50")
                    .expect("changed");
            }),
            ("no row for the file", |f, _, _, _| {
                f.store
                    .conn_for_test()
                    .execute_batch("DELETE FROM parquet_file")
                    .expect("changed");
            }),
            ("the file gone", |_, packs, name, _| {
                std::fs::remove_file(packs.path(name)).expect("removed");
            }),
            ("the file cut short", |_, packs, name, bytes| {
                std::fs::write(packs.path(name), &bytes[..bytes.len() - 1]).expect("written");
            }),
            ("its length misstated", |f, _, _, _| {
                f.store
                    .conn_for_test()
                    .execute_batch("UPDATE parquet_file SET bytes = bytes - 1")
                    .expect("changed");
            }),
        ];
        for (what, broken) in breaks {
            let mut f = fixture();
            let kept = stored(&f);
            let packs = DirPacks::new();
            with_packs(&mut f, &packs);
            let versions: Vec<VersionId> = kept.iter().map(|k| k.version).collect();
            let (id, bytes, _) = pack(&f.store, &packs, &versions, 400);
            broken(&f, &packs, &pack::name(DATE, id), &bytes);
            // A failure names the version by its place in `kept`, never by
            // its uuid: a message is written to the test's log.
            for (n, k) in kept.iter().enumerate().skip(1) {
                let got = f.store.get(&f.caller, k.uuid);
                assert_eq!(
                    got.as_ref().map(|s| &s.body),
                    Err(&StoreError::Damaged),
                    "{what}: version {n}"
                );
            }
        }
        // No pack storage at all.
        let mut f = fixture();
        let kept = stored(&f);
        let packs = DirPacks::new();
        with_packs(&mut f, &packs);
        let versions: Vec<VersionId> = kept.iter().map(|k| k.version).collect();
        pack(&f.store, &packs, &versions, 400);
        f.store.packs = None;
        assert_eq!(
            f.store.get(&f.caller, kept[3].uuid).map(|s| s.body),
            Err(StoreError::Damaged),
            "a Store with no pack storage"
        );
        // A place naming a file with no row, while the version's object is
        // still there: the catalog does not agree with itself, so this is
        // damage, never a quiet read of the object.
        let mut f = fixture();
        let kept = stored(&f);
        let packs = DirPacks::new();
        with_packs(&mut f, &packs);
        let versions: Vec<VersionId> = kept.iter().map(|k| k.version).collect();
        pack(&f.store, &packs, &versions, 400);
        for k in &kept {
            f.store.objects.put(k.version, &k.sealed).expect("put back");
        }
        f.store
            .conn_for_test()
            .execute_batch("DELETE FROM parquet_file")
            .expect("changed");
        assert_eq!(
            f.store.get(&f.caller, kept[3].uuid).map(|s| s.body),
            Err(StoreError::Damaged),
            "a place with no file row, its object there"
        );
    }

    /// A pack file's footer is read once, from its end, in at most three
    /// reads; then each row is one read of exactly its own page.
    #[test]
    fn a_packed_row_is_read_with_the_footer_once_and_then_its_page_alone() {
        let mut f = fixture();
        let kept = stored(&f);
        let packs = DirPacks::new();
        with_packs(&mut f, &packs);
        let versions: Vec<VersionId> = kept.iter().map(|k| k.version).collect();
        let (id, bytes, places) = pack(&f.store, &packs, &versions, 400);
        let name = pack::name(DATE, id);
        let footer = crate::backend::pack::tests::footer(&bytes);
        let size = bytes.len() as u64;
        packs.taken();
        for round in 0..2 {
            for (k, place) in kept.iter().zip(&places).skip(1) {
                f.store.get(&f.caller, k.uuid).expect("reads");
                let mut reads = packs.taken();
                let page = footer.page(*place).expect("its page");
                let last = reads.pop().expect("a read");
                assert_eq!(
                    last,
                    (name.clone(), page.start, page.end - page.start),
                    "round {round}: the row's own page, and no more"
                );
                if round == 0 && k.version == kept[1].version {
                    assert!(
                        (1..=3).contains(&reads.len()),
                        "the footer, in one to three reads: {reads:?}"
                    );
                    for (n, start, len) in &reads {
                        assert_eq!(n, &name, "the same file");
                        assert_eq!(start + len, size, "from the file's end");
                    }
                } else {
                    assert_eq!(reads, [], "round {round}: the footer is kept");
                }
            }
        }
    }

    /// The Store keeps at most its cap of footers: past it they are read
    /// again, and the count kept never goes over it.
    #[test]
    fn at_most_the_cap_of_footers_is_kept() {
        let mut f = fixture();
        let kept = stored(&f);
        let packs = DirPacks::new();
        with_packs(&mut f, &packs);
        f.store.footers = Footers::new(2);
        for k in &kept {
            pack(&f.store, &packs, &[k.version], usize::MAX);
        }
        for _ in 0..2 {
            for k in &kept[1..] {
                f.store.get(&f.caller, k.uuid).expect("reads");
                let held = f.store.footers.files.lock().expect("footers").len();
                assert!(held <= 2, "{held} footers kept, over the cap of 2");
            }
        }
        // A footer read ends at its file's end; a page read never does.
        let footer_reads = packs
            .taken()
            .iter()
            .filter(|(name, start, len)| Some(start + len) == packs.size(name).expect("a size"))
            .count();
        assert!(
            footer_reads > 3,
            "three files over a cap of two are read again: {footer_reads} footer reads"
        );
    }

    /// `check` lists the packed versions whose file is gone, or not the
    /// length its row says, and only those.
    #[test]
    fn check_lists_packed_versions_whose_file_is_gone_or_another_length() {
        let mut f = fixture();
        let kept = stored(&f);
        let packs = DirPacks::new();
        with_packs(&mut f, &packs);
        let (first_id, first_bytes, _) =
            pack(&f.store, &packs, &[kept[0].version, kept[1].version], 400);
        let (second, _, _) = pack(&f.store, &packs, &[kept[2].version], 400);
        let mut lost = vec![kept[0].version, kept[1].version];
        lost.sort_by_key(|v| v.0);
        let first = packs.path(&pack::name(DATE, first_id));
        std::fs::remove_file(&first).expect("removed");
        assert_eq!(f.store.check().expect("checked"), lost, "the file gone");
        std::fs::write(&first, [&first_bytes[..], b"x"].concat()).expect("written");
        assert_eq!(f.store.check().expect("checked"), lost, "a longer file");
        std::fs::write(&first, &first_bytes).expect("written");
        assert_eq!(f.store.check().expect("checked"), [], "the file back");
        std::fs::remove_file(packs.path(&pack::name(DATE, second))).expect("removed");
        assert_eq!(
            f.store.check().expect("checked"),
            [kept[2].version],
            "the other file"
        );
        f.store
            .conn_for_test()
            .execute("DELETE FROM parquet_file WHERE file_id != ?1", [second.0])
            .expect("removed");
        let mut listed = lost.clone();
        listed.push(kept[2].version);
        listed.sort_by_key(|v| v.0);
        assert_eq!(
            f.store.check().expect("checked, not refused"),
            listed,
            "a place naming a file with no row is listed with the rest"
        );
        f.store
            .conn_for_test()
            .execute(
                "INSERT INTO parquet_file (file_id, date, rows, live_rows, bytes, created_at)
                 VALUES (?1, ?2, 2, 2, ?3, 0)",
                rusqlite::params![first_id.0, DATE, first_bytes.len() as i64],
            )
            .expect("the row back");
        f.store.packs = None;
        let mut every: Vec<VersionId> = kept[..3].iter().map(|k| k.version).collect();
        every.sort_by_key(|v| v.0);
        assert_eq!(
            f.store.check().expect("checked"),
            every,
            "no pack storage: every packed version"
        );
    }

    /// Takes a tenant catalog back to before version 4: no places and no
    /// pack tables. AUTOINCREMENT's own table goes too, since only
    /// `manifest` uses it.
    pub(crate) const BEFORE_PACKS: &str = "
        DROP INDEX version_place;
        ALTER TABLE version DROP COLUMN place_file;
        ALTER TABLE version DROP COLUMN place_group;
        ALTER TABLE version DROP COLUMN place_row;
        DROP TABLE parquet_file;
        DROP TABLE purge_pending;
        DROP TABLE manifest;
        DELETE FROM sqlite_sequence;";

    /// A version-3 catalog opens at the current version with the tables
    /// and columns a new catalog has, every version unpacked and readable.
    #[test]
    fn a_version_3_catalog_is_upgraded_with_every_version_unpacked() {
        let mut f = fixture();
        let kept = stored(&f);
        let shape = |f: &Fixture| -> Vec<(String, String)> {
            let conn = f.store.conn_for_test();
            let mut rows = conn
                .prepare(
                    "SELECT m.name, p.name || ' ' || p.type || ' ' || p.\"notnull\" || ' '
                            || coalesce(p.dflt_value, '') || ' ' || p.pk
                     FROM sqlite_master m, pragma_table_info(m.name) p
                     WHERE m.type = 'table' ORDER BY m.name, p.cid",
                )
                .expect("prepare");
            let mut out: Vec<(String, String)> = rows
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .expect("query")
                .collect::<Result<_, _>>()
                .expect("rows");
            let mut indexes = conn
                .prepare("SELECT name, coalesce(sql, '') FROM sqlite_master WHERE type = 'index'")
                .expect("prepare");
            out.extend(
                indexes
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .expect("query")
                    .collect::<Result<Vec<_>, _>>()
                    .expect("rows"),
            );
            out.sort();
            out
        };
        let fresh = shape(&f);
        for table in ["parquet_file", "purge_pending", "manifest"] {
            assert!(
                fresh.iter().any(|(t, _)| t == table),
                "a new catalog has {table}"
            );
        }
        for column in ["place_file", "place_group", "place_row"] {
            assert!(
                fresh
                    .iter()
                    .any(|(t, c)| t == "version" && c.starts_with(&format!("{column} "))),
                "a new catalog's version table has {column}"
            );
        }
        f.store
            .conn_for_test()
            .execute_batch(&format!("{BEFORE_PACKS} PRAGMA user_version = 3;"))
            .expect("made version 3");
        f.reopen();
        let version: i64 = f
            .store
            .conn_for_test()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .expect("version");
        assert_eq!(
            version,
            crate::catalog::SCHEMA_VERSION,
            "the current version"
        );
        assert_eq!(shape(&f), fresh, "the same tables as a new catalog");
        for k in &kept[1..] {
            assert_eq!(
                f.store.get(&f.caller, k.uuid).expect("reads").body,
                k.body,
                "read from its object"
            );
        }
    }

    /// DuckDB, the CLI `VCON_TEST_DUCKDB` names (CI pins it by SHA-256),
    /// reads the backend's own pack files under their names: one row per
    /// packed version, the version as a UUID, the `date=` directory as a
    /// DATE, and each `sealed` value equal to the version's object.
    #[cfg(feature = "backend-tests")]
    #[test]
    fn duckdb_reads_the_pack_files_as_the_store_writes_them() {
        use crate::export::tests::duckdb::query;
        let mut f = fixture();
        let kept = stored(&f);
        let packs = DirPacks::new();
        with_packs(&mut f, &packs);
        pack(&f.store, &packs, &[kept[0].version, kept[1].version], 400);
        pack(&f.store, &packs, &[kept[2].version, kept[3].version], 400);
        let rows = query(&format!(
            "SELECT version::VARCHAR AS version, typeof(version) AS vt, typeof(date) AS dt, \
             date::VARCHAR AS date, hex(sealed) AS h \
             FROM read_parquet('{}/parquet/*/*.parquet', hive_partitioning = true) \
             ORDER BY version",
            packs.root.path().display()
        ));
        let mut expected: Vec<&Kept> = kept.iter().collect();
        expected.sort_by_key(|k| uuid::Uuid::from_bytes(k.version.0).to_string());
        assert_eq!(rows.len(), expected.len(), "one row per packed version");
        for (row, k) in rows.iter().zip(expected) {
            let hex: String = k.sealed.iter().map(|b| format!("{b:02X}")).collect();
            assert_eq!(
                row["version"],
                uuid::Uuid::from_bytes(k.version.0).to_string().as_str(),
                "the version id"
            );
            assert_eq!(row["vt"], "UUID", "read as a UUID");
            assert_eq!(row["dt"], "DATE", "the date directory as a DATE");
            assert_eq!(row["date"], DATE, "the store date");
            assert_eq!(row["h"], hex.as_str(), "the sealed blob, byte for byte");
        }
    }
}
