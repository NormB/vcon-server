#!/usr/bin/env bash
# Checks that exports of the private design documents, and the private
# spec and plan files (*.local.md), can never be staged: git must ignore
# every such name, in every export format and in any directory, no tracked
# or staged file may have one, and ordinary project files stay visible.
# Findings go to stderr: the commit hook discards stdout.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

fail=0
ignored() { git check-ignore -q --no-index -- "$1"; }

# Made-up names in the shape the exports take; the real titles are not
# named here.
stems=(
  "vcon-server — Some document"
  "vcon-server — 3. Some tab"
  "vcon-server — some-document"
  "ARCHIVED — Some document"
  "ARCHIVED — vcon-server some document"
)
formats=(pdf docx html txt md)

for stem in "${stems[@]}"; do
  for ext in "${formats[@]}"; do
    for path in "$stem.$ext" "docs/$stem.$ext" "$stem (1).$ext"; do
      if ! ignored "$path"; then
        echo "NOT IGNORED: $path" >&2
        fail=1
      fi
    done
  done
done

# The private spec and plan files, at the root and in a directory.
for path in "y.local.md" "docs/x.local.md"; do
  if ! ignored "$path"; then
    echo "NOT IGNORED: $path" >&2
    fail=1
  fi
done

# The index, so a file added with --force is caught too.
while IFS= read -r -d '' path; do
  echo "TRACKED PRIVATE FILE: $path" >&2
  fail=1
done < <(git ls-files -z -- '*.local.md')

for path in local.md docs/local-setup.md docs/x.md Cargo.toml README.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE \
  crates/vcon/src/lib.rs crates/vcon-server/src/main.rs scripts/check-ignored-exports.sh; do
  if ignored "$path"; then
    echo "WRONGLY IGNORED: $path" >&2
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  echo "check-ignored-exports: FAILED" >&2
  exit 1
fi
echo "check-ignored-exports: ok"
