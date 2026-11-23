#!/usr/bin/env bash
# Points this system's apt at a list of hosts for each package source, in
# place of the one host its image names.
#
#   bash scripts/apt-mirror-setup.sh [--root DIR] [--table FILE] [--timeout SECONDS] [--dry-run]
#
# An image's apt reads each source (an origin) from one host. When that host
# does not answer, apt's own retries ask the same host again: on 2026-11-10
# archive.ubuntu.com did not answer on port 80 and `apt-get update` in an
# ubuntu container did not end. So, before a container's first apt-get, this
# script gives apt a list of hosts for each origin the system reads, through
# apt's `mirror+file:` method: apt asks the first, and the next when it fails.
#
# A mirror that answers can still be in the middle of a sync: on 2026-11-23
# (run 45261208950) apt read jammy-security from a mirror whose files were
# not the size the release file named ("File has unexpected size ... Mirror
# sync in progress?") and the install did not end until timeout(1) ended it.
# So the script asks the origin and each of its mirrors for the release file
# (<host>/dists/<suite>/InRelease) of each suite the system reads from that
# origin, all at once and each within the time limit, and keeps a mirror
# only when each of its files is byte for byte the origin's. A mirror that
# differs is left out and named in the log. For an origin that serves a
# -security suite, the origin itself comes first in the list.
#
# When the origin does not answer (the 2026-11-10 case), the mirrors that
# answer are compared with one another: a mirror whose copy of a suite's
# file has an older `Date:` than another mirror's copy is left out; of the
# others, those whose files agree are kept, the largest such group, or the
# group listed first when two are as large. When no two agree, only the
# first is kept, and the log says so.
#
# The table is scripts/apt-mirrors.txt; each origin is among its own
# mirrors. scripts/docs_test.py reads the table for the how-to runner and
# runs this script with --dry-run to choose its mirrors, so both choose by
# this one rule. The files written are the ones that runner's
# apt_mirror_setup and APT_RETRIES write: scripts/test_apt_mirror_setup.py
# holds the two to the same result.
#
# It needs bash, coreutils, grep and sed and nothing else: an image has no
# curl and no python3 before its first install. The questions are asked
# over bash's /dev/tcp, so a mirror is an http:// address.
#
# With --dry-run it asks every question, prints what it would do, and
# writes nothing.
#
# Exit codes: 0 the sources read their lists (or already did); 2 the
# command line or the table is wrong; 3 no mirror of an origin answered, or
# none holds the newest release file of every suite, and nothing was
# changed; 4 no source of this system is in the table, and nothing was
# changed.
set -euo pipefail

me=apt-mirror-setup

# How often apt asks again for a download that failed, and the seconds it
# waits on a host that sends nothing: docs_test.py's APT_TRIES and
# APT_TIMEOUT, where the reason for the small numbers is written.
tries=3
patience=10

# The file at the http:// address $1, on stdout, when its host answers 200.
# Run under timeout(1) by the caller: bash has no time limit of its own for
# a connection.
fetch() {
    local rest=${1#http://} host path port=80 line
    host=${rest%%/*}
    path=/
    if [ "$rest" != "$host" ]; then
        path=/${rest#*/}
    fi
    if [ "$host" != "${host%:*}" ]; then
        port=${host##*:}
    fi
    exec 3<>"/dev/tcp/${host%:*}/$port" || return 1
    printf 'GET %s HTTP/1.0\r\nHost: %s\r\nUser-Agent: %s\r\nAccept: */*\r\n\r\n' "$path" "$host" "$me" >&3
    IFS= read -r line <&3 || return 1
    case $line in
        "HTTP/1.0 200 "* | "HTTP/1.1 200 "* | "HTTP/1.0 200"$'\r' | "HTTP/1.1 200"$'\r') ;;
        *) return 1 ;;
    esac
    # The headers end at an empty line; the file follows it. bash reads a
    # socket a byte at a time, so cat gets the rest whole.
    while IFS= read -r line <&3; do
        if [ -z "${line%$'\r'}" ]; then
            cat <&3
            return 0
        fi
    done
    return 1
}

if [ "${1:-}" = "--fetch" ]; then
    fetch "$2" 2>/dev/null
    exit
fi

root=""
table="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/apt-mirrors.txt"
limit=5
dry=no
while [ $# -gt 0 ]; do
    case $1 in
        --dry-run)
            dry=yes
            shift
            ;;
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
            echo "$me: $1 is not an option (--root DIR, --table FILE, --timeout SECONDS, --dry-run)" >&2
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
    listed=no
    for m in $mirrors; do
        case $m in
            http://?*) ;;
            *)
                echo "$me: $table:$n: $m is not an http:// address" >&2
                exit 2
                ;;
        esac
        if [ "$m" = "$origin" ]; then
            listed=yes
        fi
    done
    # Each mirror is held to the origin's own files.
    if [ "$listed" = no ]; then
        echo "$me: $table:$n: $origin is not among its own mirrors" >&2
        exit 2
    fi
    origins+=("$origin")
    mirrors_of+=("$mirrors")
done 4<"$table"

# An origin as sed finds it: with or without its last slash, as a whole
# word, since one origin (.../debian) is the start of another
# (.../debian-security).
pattern() {
    printf '%s/\\?\\([[:space:]]\\|$\\)' "$(printf '%s' "${1%/}" | sed 's/[].[*^$\#]/\\&/g')"
}

# The suites the sources read from the origin `want`, into `got_suites`,
# in the order the files name them: from one-line entries
# (`deb [options] <address> <suite> <components>`) and from deb822
# stanzas (their `URIs:` and `Suites:` fields). A line that is a comment
# names nothing. A suite named twice is asked for once all the same: each
# host is asked once for each suite.
end_stanza() {
    local u
    for u in "${uris[@]}"; do
        if [ "${u%/}" = "$want" ]; then
            got_suites+=("${suites[@]}")
            break
        fi
    done
    uris=()
    suites=()
}
suites_read_from() {
    local f line i
    local -a words
    want=${1%/}
    got_suites=()
    for f in "${sources[@]}"; do
        uris=()
        suites=()
        while IFS= read -r line || [ -n "$line" ]; do
            if [[ $line =~ ^[[:space:]]*# ]]; then
                continue
            fi
            line=${line%%#*}
            read -r -a words <<<"$line" || true
            if [ ${#words[@]} -eq 0 ]; then
                end_stanza
                continue
            fi
            case ${words[0],,} in
                deb | deb-src)
                    i=1
                    if [[ ${words[1]:-} == \[* ]]; then
                        while [ "$i" -lt ${#words[@]} ] && [[ ${words[$i]} != *\] ]]; do
                            i=$((i + 1))
                        done
                        i=$((i + 1))
                    fi
                    if [ $((i + 1)) -lt ${#words[@]} ] && [ "${words[$i]%/}" = "$want" ]; then
                        got_suites+=("${words[$((i + 1))]}")
                    fi
                    ;;
                uris:) uris=("${words[@]:1}") ;;
                suites:) suites=("${words[@]:1}") ;;
            esac
        done <"$f"
        end_stanza
    done
}
uris=()
suites=()

# Each origin this system reads, with the suites it reads from it.
read_origins=()
read_suites=()
for i in "${!origins[@]}"; do
    suites_read_from "${origins[$i]}"
    if [ ${#got_suites[@]} -gt 0 ]; then
        read_origins+=("$i")
        read_suites+=("${got_suites[*]}")
    fi
done

if [ ${#read_origins[@]} -eq 0 ]; then
    if [ ${#sources[@]} -gt 0 ] && grep -q -F -- "mirror+file:$root/etc/apt/docs-test-mirrors-" "${sources[@]}"; then
        echo "$me: this system's sources already read their mirror lists"
        exit 0
    fi
    echo "$me: no package source of this system is in $table: its apt would read each source from one host" >&2
    exit 4
fi

# Every release file asked for at once, each once, into its own file.
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
declare -A file_of=() pid_of=()
k=0
for j in "${!read_origins[@]}"; do
    for m in ${mirrors_of[${read_origins[$j]}]}; do
        for s in ${read_suites[$j]}; do
            key="$m $s"
            if [ -z "${file_of[$key]:-}" ]; then
                k=$((k + 1))
                file_of[$key]=$work/$k
                timeout "$limit" bash "${BASH_SOURCE[0]}" --fetch "${m%/}/dists/$s/InRelease" </dev/null >"$work/$k" 2>/dev/null &
                pid_of[$key]=$!
            fi
        done
    done
done

# Of each copy that came: its SHA-256, and its `Date:` in seconds (0 for a
# copy without one, the oldest).
declare -A hash_of=() date_of=()
for key in "${!pid_of[@]}"; do
    if wait "${pid_of[$key]}" && [ -s "${file_of[$key]}" ]; then
        read -r sum _ < <(sha256sum "${file_of[$key]}")
        hash_of[$key]=$sum
        stamp=$(grep -m 1 '^Date:' "${file_of[$key]}" || true)
        date_of[$key]=0
        if [ -n "$stamp" ]; then
            date_of[$key]=$(date -u -d "${stamp#Date:}" +%s 2>/dev/null) || date_of[$key]=0
        fi
    fi
done

# The SHA-256 of each copy the host $1 serves of the suites in `ss`; a
# failure when it served no copy of one of them.
copies() {
    local s out=""
    for s in "${ss[@]}"; do
        if [ -z "${hash_of[$1 $s]:-}" ]; then
            return 1
        fi
        out="$out ${hash_of[$1 $s]}"
    done
    printf '%s\n' "$out"
}

# The first suite in `ss` whose copy on the host $1 is not the host $2's.
first_other() {
    local s
    for s in "${ss[@]}"; do
        if [ "${hash_of[$1 $s]}" != "${hash_of[$2 $s]:-}" ]; then
            printf '%s\n' "$s"
            return
        fi
    done
}

found=()
chosen=()
for j in "${!read_origins[@]}"; do
    i=${read_origins[$j]}
    origin=${origins[$i]}
    read -r -a ss <<<"${read_suites[$j]}"
    read -r -a ms <<<"${mirrors_of[$i]}"
    kept=()
    if official=$(copies "$origin"); then
        for m in "${ms[@]}"; do
            if ! mine=$(copies "$m"); then
                continue
            fi
            if [ "$mine" = "$official" ]; then
                kept+=("$m")
            else
                echo "$me: left out $m: its InRelease for $(first_other "$m" "$origin") is not the one $origin serves (a mirror in the middle of a sync)"
            fi
        done
        # A security fix is read from its own host first.
        for s in "${ss[@]}"; do
            if [ "$s" != "${s%-security}" ]; then
                rest=()
                for m in "${kept[@]}"; do
                    if [ "$m" != "$origin" ]; then
                        rest+=("$m")
                    fi
                done
                kept=("$origin" ${rest[@]+"${rest[@]}"})
                break
            fi
        done
        # One host left of several: say so, beside the hosts named above.
        if [ ${#kept[@]} -eq 1 ] && [ ${#ms[@]} -gt 1 ]; then
            echo "$me: only $origin itself remains for its packages: each of its other mirrors is out of sync or did not answer, so a failure of that one host fails the install"
        fi
    else
        echo "$me: $origin could not be reached: its mirrors are compared with one another, and only those that hold its newest InRelease are kept"
        answering=()
        for m in "${ms[@]}"; do
            if copies "$m" >/dev/null; then
                answering+=("$m")
            fi
        done
        if [ ${#answering[@]} -eq 0 ]; then
            echo "$me: no package mirror answered for $origin (tried ${mirrors_of[$i]// /, }): the package mirrors are unreachable from this machine" >&2
            exit 3
        fi
        # Never a copy older than another mirror's.
        unset newest
        declare -A newest=()
        for s in "${ss[@]}"; do
            newest[$s]=0
            for m in "${answering[@]}"; do
                if [ "${date_of[$m $s]}" -gt "${newest[$s]}" ]; then
                    newest[$s]=${date_of[$m $s]}
                fi
            done
        done
        current=()
        for m in "${answering[@]}"; do
            behind=""
            for s in "${ss[@]}"; do
                if [ "${date_of[$m $s]}" -lt "${newest[$s]}" ]; then
                    behind=$s
                    break
                fi
            done
            if [ -n "$behind" ]; then
                echo "$me: left out $m: its InRelease for $behind is older than another mirror's"
            else
                current+=("$m")
            fi
        done
        if [ ${#current[@]} -eq 0 ]; then
            echo "$me: no mirror of $origin holds the newest InRelease of every suite (tried ${mirrors_of[$i]// /, }): apt would read some of its files from an older copy" >&2
            exit 3
        fi
        # The largest group that agrees, the one listed first on a tie.
        best=""
        most=0
        for m in "${current[@]}"; do
            mine=$(copies "$m")
            count=0
            for o in "${current[@]}"; do
                if [ "$(copies "$o")" = "$mine" ]; then
                    count=$((count + 1))
                fi
            done
            if [ "$count" -gt "$most" ]; then
                best=$m
                most=$count
            fi
        done
        if [ "$most" -lt 2 ]; then
            echo "$me: no two mirrors of $origin hold the same newest InRelease: packages come from $best alone, the one with the newest copy"
        fi
        agreed=$(copies "$best")
        for m in "${current[@]}"; do
            if [ "$(copies "$m")" = "$agreed" ]; then
                kept+=("$m")
            else
                echo "$me: left out $m: its InRelease for $(first_other "$m" "$best") is not the copy $best serves"
            fi
        done
    fi
    found+=("$origin")
    chosen+=("${kept[*]}")
done

# Every question is answered: only now is anything written.
if [ "$dry" = no ]; then
    mkdir -p "$root/etc/apt/apt.conf.d"
    printf 'Acquire::Retries "%s";\nAcquire::http::Timeout "%s";\nAcquire::https::Timeout "%s";\n' \
        "$tries" "$patience" "$patience" >"$root/etc/apt/apt.conf.d/80docs-test-retries"
fi
for i in "${!found[@]}"; do
    origin=${found[$i]}
    listing="$root/etc/apt/docs-test-mirrors-$i.list"
    if [ "$dry" = no ]; then
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
    fi
    echo "$me: packages from $origin come from ${chosen[$i]// /, then }"
done
