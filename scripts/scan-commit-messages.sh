#!/usr/bin/env bash
# Scans the whole stored message (%B) of each commit named, every line,
# with the same rules the commit-msg hook applies (scripts/scan_forbidden.py
# --commit-message): no AI attribution, no personal data, no private
# design-document ids, and no email address in a trailer.
# The push hook runs it on every outgoing commit and CI on a pull request's
# commits, so a commit made with --no-verify or by another tool is caught.
#
# Usage: scripts/scan-commit-messages.sh <commit>...
# Exits 1, naming each commit and its findings, when any message has one.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bad=0
for commit in "$@"; do
  message="$(git log -1 --format=%B "$commit")" || { echo "scan-commit-messages: cannot read $commit" >&2; exit 1; }
  if ! findings="$(printf '%s\n' "$message" | python3 -I "$here/scan_forbidden.py" --commit-message - 2>&1)"; then
    echo "scan-commit-messages: refused: $commit" >&2
    printf '%s\n' "$findings" >&2
    bad=1
  fi
done
exit "$bad"
