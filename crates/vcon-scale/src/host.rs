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
/// followed.
///
/// # Errors
///
/// The first error reading a directory or a file's metadata.
pub fn tree_bytes(root: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                total = total.saturating_add(entry.metadata()?.len());
            }
        }
    }
    Ok(total)
}
