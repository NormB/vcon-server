//! The reader of a table on a docs page, in one place: tests/formats.rs
//! (on-disk-format.md), tests/env_contract.rs (stability.md) and
//! tests/audit_contract.rs (audit-log-records.md) bring it in by `#[path]`.

/// The rows of the first table after `heading` in `text`: each row's cells,
/// trimmed, the header row first and the separator row left out. None when
/// the heading or its table is not there.
pub fn table(text: &str, heading: &str) -> Option<Vec<Vec<String>>> {
    let mut lines = text.lines().skip_while(|l| l.trim() != heading).skip(1);
    let mut rows = Vec::new();
    let mut started = false;
    for line in lines.by_ref() {
        let line = line.trim();
        if line.starts_with('|') {
            started = true;
            let cells: Vec<String> = line
                .trim_matches('|')
                .split('|')
                .map(|c| c.trim().to_owned())
                .collect();
            if !cells
                .iter()
                .all(|c| !c.is_empty() && c.chars().all(|x| x == '-' || x == ':'))
            {
                rows.push(cells);
            }
        } else if started || line.starts_with("## ") {
            break;
        }
    }
    (!rows.is_empty()).then_some(rows)
}
