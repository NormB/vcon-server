//! A catalog keeps temporary tables and sorts in memory, so no plain
//! temporary file is written. SQLite unlinks a temporary file as soon as it
//! opens it, so the test looks for open files in SQLITE_TMPDIR rather than
//! names. Setting the environment is process-wide, so this file is its own
//! test binary with one test.

use std::path::Path;

use vcon_server::catalog;
use vcon_server::envelope::Key;

/// A temporary table too big for the page cache.
const FILL: &str = "PRAGMA cache_size = 8;
    CREATE TEMP TABLE big (x BLOB);
    WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 20000)
    INSERT INTO big SELECT randomblob(1000) FROM n;";

/// A sort too big for memory unless temporary data stays in memory.
const SORT: &str = "SELECT x FROM big ORDER BY x";

/// The files this process has open under `dir`, deleted or not.
fn open_files_under(dir: &Path) -> Vec<String> {
    std::fs::read_dir("/proc/self/fd")
        .expect("list /proc/self/fd")
        .filter_map(|fd| std::fs::read_link(fd.ok()?.path()).ok())
        .map(|target| target.to_string_lossy().into_owned())
        .filter(|target| target.starts_with(&*dir.to_string_lossy()))
        .collect()
}

/// Fills a temporary table on `conn`, sorts it, and returns the files open
/// under `tmp` while the sort's first row is held.
fn files_while_sorting(conn: &rusqlite::Connection, tmp: &Path) -> Vec<String> {
    conn.execute_batch(FILL).expect("fill a temporary table");
    let mut sort = conn.prepare(SORT).expect("prepare");
    let mut rows = sort.query([]).expect("sort");
    rows.next().expect("first row").expect("a row");
    open_files_under(tmp)
}

#[test]
fn sorts_and_temporary_tables_write_no_temporary_file() {
    let tmp = tempfile::tempdir().expect("temp dir");
    // SAFETY: this binary runs one test, and nothing else reads the
    // environment while it is set.
    unsafe { std::env::set_var("SQLITE_TMPDIR", tmp.path()) };

    // The check can see a temporary file: a plain connection told to use
    // files has one open under SQLITE_TMPDIR while it sorts.
    let control = rusqlite::Connection::open_in_memory().expect("control");
    control
        .pragma_update(None, "temp_store", "FILE")
        .expect("temp_store = FILE");
    assert!(
        !files_while_sorting(&control, tmp.path()).is_empty(),
        "the control writes a temporary file"
    );
    drop(control);

    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    let conn = catalog::open(
        &dir.path().join("catalog.sqlite"),
        &key,
        "CREATE TABLE note (text TEXT NOT NULL);",
    )
    .expect("opens");
    assert_eq!(files_while_sorting(&conn, tmp.path()), Vec::<String>::new());
}
