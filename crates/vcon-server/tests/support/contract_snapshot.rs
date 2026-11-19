// The message of a contract snapshot that differs from what the code gives,
// in one place: tests/cli_contract.rs (contract/cli.json) and the lib test
// config::tests::contract (contract/config.json) include this file's text
// (the include macro, not a module path), so it holds items only and its
// comments are not inner docs.

/// The message of a snapshot whose text `file` is not `now`, the text
/// `source` gives: the first line that differs, quoted from both sides,
/// and the one command, `regenerate`, that writes `snapshot` again.
pub fn stale(snapshot: &str, source: &str, regenerate: &str, file: &str, now: &str) -> String {
    let (mut old, mut new) = (file.lines(), now.lines());
    let mut n = 1;
    loop {
        match (old.next(), new.next()) {
            (Some(a), Some(b)) if a == b => n += 1,
            (a, b) => {
                return format!(
                    "{snapshot} is not what {source}; line {n} is `{}` in the file and `{}` now. \
                     Write it again with `{regenerate}`, check the diff, and commit it",
                    a.unwrap_or("(none)"),
                    b.unwrap_or("(none)")
                );
            }
        }
    }
}
