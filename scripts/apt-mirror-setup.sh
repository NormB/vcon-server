#!/usr/bin/env bash
# Points this system's apt at a list of hosts for each package source, in
# place of the one host its image names.
#
#   bash scripts/apt-mirror-setup.sh [--root DIR] [--table FILE] [--timeout SECONDS]
#
# An image's apt reads each source (an origin) from one host. When that host
# does not answer, apt's own retries ask the same host again: on 2026-11-10
# archive.ubuntu.com did not answer on port 80 and `apt-get update` in an
# ubuntu container did not end. So, before a container's first apt-get, this
# script asks each mirror of each origin the system reads for its index of
# suites (<mirror>/dists/), keeps those that answer 200 within the time
# limit, in the table's order, and gives apt that list through its
# `mirror+file:` method: apt asks the first, and the next when it fails.
#
# The table is scripts/apt-mirrors.txt, which scripts/docs_test.py reads for
# the how-to runner too. The files written are the ones that runner's
# apt_mirror_setup and APT_RETRIES write: scripts/test_apt_mirror_setup.py
# holds the two to the same result.
#
# It needs bash and coreutils and nothing else: an image has no curl and no
# python3 before its first install. The question is asked over bash's
# /dev/tcp, so a mirror is an http:// address.
#
# Exit codes: 0 the sources read their lists (or already did); 2 the
# command line or the table is wrong; 3 no mirror of an origin answered,
# and nothing was changed; 4 no source of this system is in the table, and
# nothing was changed.
set -euo pipefail

me=apt-mirror-setup

# How often apt asks again for a download that failed, and the seconds it
# waits on a host that sends nothing: docs_test.py's APT_TRIES and
# APT_TIMEOUT, where the reason for the small numbers is written.
tries=3
patience=10

# Whether the mirror at $1 serves its index of suites. Run under timeout(1)
# by the caller: bash has no time limit of its own for a connection.
probe() {
    local rest=${1#http://} host path port=80 line
    host=${rest%%/*}
    path=/
    if [ "$rest" != "$host" ]; then
        path=/${rest#*/}
    fi
    path=${path%/}/dists/
    if [ "$host" != "${host%:*}" ]; then
        port=${host##*:}
    fi
    exec 3<>"/dev/tcp/${host%:*}/$port" || return 1
    printf 'GET %s HTTP/1.0\r\nHost: %s\r\nUser-Agent: %s\r\nAccept: */*\r\n\r\n' "$path" "$host" "$me" >&3
    IFS= read -r line <&3 || true
    case $line in
        "HTTP/1.0 200 "* | "HTTP/1.1 200 "* | "HTTP/1.0 200"$'\r' | "HTTP/1.1 200"$'\r') return 0 ;;
    esac
    return 1
}

if [ "${1:-}" = "--probe" ]; then
    probe "$2" 2>/dev/null
    exit
fi

root=""
table="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/apt-mirrors.txt"
limit=5
while [ $# -gt 0 ]; do
    case $1 in
        --root | --table | --timeout)
            if [ $# -lt 2 ]; then
                echo "$me: $1 needs a value" >&2
                exit 2
            fi
            case $1 in
                --root) root=$2 ;;
                --table) table=$2 ;;
                --timeout) limit=$2 ;;
            esac
            shift 2
            ;;
        *)
            echo "$me: $1 is not an option (--root DIR, --table FILE, --timeout SECONDS)" >&2
            exit 2
            ;;
    esac
done
case $limit in
    '' | *[!0-9]*)
        echo "$me: --timeout takes a whole number of seconds, not $limit" >&2
        exit 2
        ;;
esac
if [ ! -f "$table" ]; then
    echo "$me: the table of mirrors $table is not there" >&2
    exit 2
fi

sources=()
for f in "$root/etc/apt/sources.list" "$root/etc/apt/sources.list.d"/*; do
    if [ -f "$f" ]; then
        sources+=("$f")
    fi
done

# The table, whole, before any question is asked: a table that is wrong is
# reported as that, whatever this system reads.
origins=()
mirrors_of=()
n=0
while IFS= read -r line <&4 || [ -n "$line" ]; do
    n=$((n + 1))
    read -r origin mirrors <<<"$line" || true
    case $origin in
        '' | '#'*) continue ;;
    esac
    if [ -z "$mirrors" ]; then
        echo "$me: $table:$n: $origin has no mirror" >&2
        exit 2
    fi
    for m in $mirrors; do
        case $m in
            http://?*) ;;
            *)
                echo "$me: $table:$n: $m is not an http:// address" >&2
                exit 2
                ;;
        esac
    done
    origins+=("$origin")
    mirrors_of+=("$mirrors")
done 4<"$table"

# An origin as sed and grep find it: with or without its last slash, as a
# whole word, since one origin (.../debian) is the start of another
# (.../debian-security).
pattern() {
    printf '%s/\\?\\([[:space:]]\\|$\\)' "$(printf '%s' "${1%/}" | sed 's/[].[*^$\#]/\\&/g')"
}

declare -A answers=()
found=()
chosen=()
for i in "${!origins[@]}"; do
    origin=${origins[$i]}
    if [ ${#sources[@]} -eq 0 ] || ! grep -q -- "$(pattern "$origin")" "${sources[@]}"; then
        continue
    fi
    kept=""
    for m in ${mirrors_of[$i]}; do
        if [ -z "${answers[$m]:-}" ]; then
            if timeout "$limit" bash "${BASH_SOURCE[0]}" --probe "$m" </dev/null; then
                answers[$m]=yes
            else
                answers[$m]=no
            fi
        fi
        if [ "${answers[$m]}" = yes ]; then
            kept="$kept $m"
        fi
    done
    if [ -z "$kept" ]; then
        echo "$me: no package mirror answered for $origin (tried ${mirrors_of[$i]// /, }): the package mirrors are unreachable from this machine" >&2
        exit 3
    fi
    found+=("$origin")
    chosen+=("${kept# }")
done

if [ ${#found[@]} -eq 0 ]; then
    if [ ${#sources[@]} -gt 0 ] && grep -q -F -- "mirror+file:$root/etc/apt/docs-test-mirrors-" "${sources[@]}"; then
        echo "$me: this system's sources already read their mirror lists"
        exit 0
    fi
    echo "$me: no package source of this system is in $table: its apt would read each source from one host" >&2
    exit 4
fi

# Every question is answered: only now is anything written.
mkdir -p "$root/etc/apt/apt.conf.d"
printf 'Acquire::Retries "%s";\nAcquire::http::Timeout "%s";\nAcquire::https::Timeout "%s";\n' \
    "$tries" "$patience" "$patience" >"$root/etc/apt/apt.conf.d/80docs-test-retries"
for i in "${!found[@]}"; do
    origin=${found[$i]}
    listing="$root/etc/apt/docs-test-mirrors-$i.list"
    # Each with its place as its priority: among mirrors without one apt's
    # mirror method chooses for itself, not in the order written.
    place=0
    for m in ${chosen[$i]}; do
        place=$((place + 1))
        printf '%s\tpriority:%s\n' "$m" "$place"
    done >"$listing"
    for f in "${sources[@]}"; do
        sed -i "s#$(pattern "$origin")#mirror+file:$listing\\1#" "$f"
    done
    echo "$me: packages from $origin come from ${chosen[$i]// /, then }"
done
