//! The licence texts sit in the repository root as `LICENSE-APACHE` and
//! `LICENSE-MIT`, and again in `LICENSES/` named by SPDX identifier, the
//! layout of the REUSE specification. The copies must stay the same texts,
//! and `LICENSES/` must hold no licence the root does not.

use std::path::{Path, PathBuf};

/// The repository root, two levels above this crate.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Each file in `LICENSES/` and the root file it copies.
const COPIES: [(&str, &str); 2] = [
    ("Apache-2.0.txt", "LICENSE-APACHE"),
    ("MIT.txt", "LICENSE-MIT"),
];

#[test]
fn each_licence_in_licenses_is_the_root_text() {
    for (copy, original) in COPIES {
        let copy_path = root().join("LICENSES").join(copy);
        let copied = std::fs::read(&copy_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", copy_path.display()));
        let text = std::fs::read(root().join(original)).expect("read the root licence");
        assert!(copied == text, "LICENSES/{copy} differs from {original}");
    }
}

#[test]
fn licenses_holds_only_the_two_licences() {
    let dir = root().join("LICENSES");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("list {}: {e}", dir.display()))
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["Apache-2.0.txt", "MIT.txt"],
        "the crate is MIT OR Apache-2.0, so LICENSES holds those two texts and nothing else"
    );
}

/// Whether a root file name is one the OpenSSF Best Practices site takes as
/// the licence statement: its detector matches
/// `/\A([A-Za-z0-9]+-)?(license|copying)(\.md|\.txt)?\Z/i`. A prefix is
/// allowed and a suffix is not, so `LICENSE-MIT` and `LICENSE-APACHE` never
/// match.
fn best_practices_licence_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let stem = lower
        .strip_suffix(".md")
        .or_else(|| lower.strip_suffix(".txt"))
        .unwrap_or(&lower);
    let base = match stem.split_once('-') {
        Some((prefix, rest))
            if !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_alphanumeric()) =>
        {
            rest
        }
        _ => stem,
    };
    base == "license" || base == "copying"
}

/// The site ignores a matching file shorter than this (NONTRIVIAL_MIN_SIZE).
const BEST_PRACTICES_MIN_SIZE: u64 = 40;

/// The names in the repository root.
fn root_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root())
        .expect("list the repository root")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn the_best_practices_name_rule_is_the_sites() {
    for name in [
        "DUAL-LICENSE",
        "LICENSE",
        "license.md",
        "COPYING.txt",
        "MIT-license",
    ] {
        assert!(
            best_practices_licence_name(name),
            "{name} matches the site's pattern"
        );
    }
    for name in [
        "LICENSE-MIT",
        "LICENSE-APACHE",
        "LICENSES",
        "-LICENSE",
        "a-b-LICENSE",
        "LICENSE.rst",
    ] {
        assert!(
            !best_practices_licence_name(name),
            "{name} does not match the site's pattern"
        );
    }
}

#[test]
fn the_root_has_a_licence_file_the_best_practices_site_finds() {
    let found: Vec<(String, u64)> = root_names()
        .into_iter()
        .filter(|name| best_practices_licence_name(name))
        .map(|name| {
            let len = std::fs::metadata(root().join(&name))
                .unwrap_or_else(|e| panic!("stat {name}: {e}"))
                .len();
            (name, len)
        })
        .collect();
    assert!(
        found.iter().any(|(_, len)| *len >= BEST_PRACTICES_MIN_SIZE),
        "no root file both matches the Best Practices licence pattern and has \
         {BEST_PRACTICES_MIN_SIZE} bytes or more; found {found:?}"
    );
}

#[test]
fn the_root_has_no_plain_licence_file() {
    // GitHub's licensee scores a plain LICENSE above LICENSE-MIT and
    // LICENSE-APACHE, and reads a dual-licence notice there as NOASSERTION.
    let plain = ["license", "license.md", "license.txt", "copying"];
    let names: Vec<String> = root_names()
        .into_iter()
        .filter(|name| plain.contains(&name.to_ascii_lowercase().as_str()))
        .collect();
    assert!(names.is_empty(), "the root must not hold {names:?}");
}

/// The value of the first line `<key> = "<value>"` in TOML text, or of the
/// first line `<key>: <value>` in a text file.
fn field<'a>(text: &'a str, key: &str, separator: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let rest = line
            .trim()
            .strip_prefix(key)?
            .trim_start()
            .strip_prefix(separator)?;
        Some(rest.trim().trim_matches('"'))
    })
}

#[test]
fn the_dual_licence_file_names_the_crates_licence() {
    let cargo = std::fs::read_to_string(root().join("Cargo.toml")).expect("read Cargo.toml");
    let expected = field(&cargo, "license", "=").expect("Cargo.toml has a license");
    let path = root().join("DUAL-LICENSE");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert_eq!(
        field(&text, "SPDX-License-Identifier", ":"),
        Some(expected),
        "DUAL-LICENSE's SPDX expression is Cargo.toml's license"
    );
}
