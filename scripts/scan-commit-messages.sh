#!/usr/bin/env bash
# Scans the whole stored message (%B) of each commit named, every line,
# with the same rules the commit-msg hook applies (scripts/scan_forbidden.py):
# no AI attribution, no personal data, no private design-document ids.
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
  if ! findings="$(printf '%s\n' "$message" | sed 's/^/<commit message>\t/' | python3 -I "$here/scan_forbidden.py")"; then
    echo "scan-commit-messages: refused: $commit" >&2
    printf '%s\n' "$findings" >&2
    bad=1
  fi
done
exit "$bad"
