#!/usr/bin/env bash
# Checks that exports of the private design documents can never be staged:
# git must ignore every export name, in every export format and in any
# directory, while ordinary project files stay visible.
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
        echo "NOT IGNORED: $path"
        fail=1
      fi
    done
  done
done

for path in Cargo.toml README.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE \
  crates/vcon-form/src/lib.rs crates/vcon-server/src/main.rs scripts/check-ignored-exports.sh; do
  if ignored "$path"; then
    echo "WRONGLY IGNORED: $path"
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  echo "check-ignored-exports: FAILED"
  exit 1
fi
echo "check-ignored-exports: ok"
