#!/usr/bin/env bash
# apt-get, with `apt-get update` run again after a wait when a mirror was in
# the middle of a sync. Every other apt-get command runs once, as given.
#
#   apt-get-retry.sh [apt-get arguments]
#
# On 2026-11-26 main's run 45805685940 ("how-to release-ubuntu against a
# live server") failed at docs/install.md's `apt-get update`: "File has
# unexpected size (1513652 != 1513649). Mirror sync in progress?" from
# security.ubuntu.com. scripts/apt-mirror-setup.sh had left out every
# other mirror of that origin, as their release files were not the
# origin's, so apt had no other host to ask, and its own retries
# (Acquire::Retries) ask again at once, before the sync can end. The same
# job had passed on the pull request's head.
#
# So an update whose run fails and whose errors hold apt's text for a mirror
# in the middle of a sync ("Mirror sync in progress", or "Hash Sum
# mismatch") is run again after a wait: by default 15, 30 and 60 seconds,
# four runs in about two minutes, each wait logged. A failure of any other
# kind ends at once with apt-get's exit code, as does the last run's. An
# install is never run again: its index files are the update's.
#
# The how-to runner (scripts/docs_test.py, apt_get_retry_install) installs
# this file as /usr/local/sbin/apt-get in each container whose apt it sets
# up, where root's PATH finds it before /usr/bin/apt-get, so a page's own
# `apt-get update` runs as a reader types it. The release's smoke job
# installs it the same way. scripts/test_apt_get_retry.py drives the rule
# with a fake apt-get.
#
# It needs bash and coreutils only: an image has no python3 before its
# first install.
#
# Settings, for tests:
#   APT_GET_REAL          the apt-get to run (default /usr/bin/apt-get)
#   APT_UPDATE_DELAYS     the waits in seconds, separated by spaces
#                         (default "15 30 60"; empty: no second run)
#   APT_GET_RETRY_SLEEP   the command that waits (default sleep)
#
# Exit codes: apt-get's; 2 when APT_UPDATE_DELAYS is not whole numbers.
set -uo pipefail

me=apt-get-retry
real=${APT_GET_REAL:-/usr/bin/apt-get}
delays=${APT_UPDATE_DELAYS-15 30 60}
sleeper=${APT_GET_RETRY_SLEEP:-sleep}

# Split on spaces, with no file name expansion of a word.
set -f
# shellcheck disable=SC2206
waits=($delays)
set +f
for w in "${waits[@]}"; do
    case $w in
        '' | *[!0-9]*)
            echo "$me: APT_UPDATE_DELAYS must be whole numbers of seconds, not \"$delays\"" >&2
            exit 2 ;;
    esac
done

update=false
for arg in "$@"; do
    if [ "$arg" = update ]; then
        update=true
        break
    fi
done
if ! $update; then
    exec "$real" "$@"
fi

# Whether the errors in file $1 are apt's for a mirror in the middle of a
# sync.
mid_sync() {
    local text
    text=$(< "$1")
    [[ $text == *"Mirror sync in progress"* || $text == *"Hash Sum mismatch"* ]]
}

errors=$(mktemp) || exit 1
trap 'rm -f "$errors"' EXIT
tries=$((${#waits[@]} + 1))
try=1
while :; do
    # stdout passes through as it comes; stderr goes to the caller's stderr
    # and to $errors, which is read only after the run.
    { "$real" "$@" 2>&1 1>&3 3>&- | tee "$errors" >&2; } 3>&1 < /dev/null
    code=${PIPESTATUS[0]}
    if [ "$code" -eq 0 ] || [ "$code" -ne 100 ] || ! mid_sync "$errors"; then
        exit "$code"
    fi
    if [ "$try" -ge "$tries" ]; then
        echo "$me: a mirror was still in the middle of a sync after $tries tries" >&2
        exit "$code"
    fi
    wait=${waits[$((try - 1))]}
    echo "$me: a mirror was in the middle of a sync (try $try of $tries); the update runs again in $wait s" >&2
    "$sleeper" "$wait"
    try=$((try + 1))
done
