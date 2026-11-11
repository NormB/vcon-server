#!/usr/bin/env bash
# Waits until an object store is ready: until it answers a signed request
# for its list of buckets with 200.
#
#   scripts/wait-for-s3.sh <name> [<log file>]
#
# with VCON_TEST_S3_ENDPOINT, VCON_TEST_S3_ACCESS_KEY and
# VCON_TEST_S3_SECRET_KEY set. <name> is what the messages call the store.
#
# A store listens before it can serve. MinIO answers every request with 503
# (XMinioServerNotInitialized) until it is initialized; on 2026-11-11 CI's
# MinIO job waited for the first answer of any status, went on, and failed
# at its first write with that 503. The first answer says the port is open.
# A signed request answered with 200 says what the next step needs: the
# store serves, and it takes the key.
#
# It asks once a second for 60 seconds (WAIT_FOR_S3_TRIES and
# WAIT_FOR_S3_INTERVAL change the two numbers, for the tests). Exit codes: 0
# ready; 1 never ready, with the last status, and the store's log if a file
# was named; 2 the command line or a setting is wrong.
set -euo pipefail

me=wait-for-s3
if [ $# -lt 1 ] || [ $# -gt 2 ]; then
    echo "usage: wait-for-s3.sh <name> [<log file>]" >&2
    exit 2
fi
name=$1
log=${2-}
for setting in VCON_TEST_S3_ENDPOINT VCON_TEST_S3_ACCESS_KEY VCON_TEST_S3_SECRET_KEY; do
    if [ -z "${!setting-}" ]; then
        echo "$me: $setting is not set" >&2
        exit 2
    fi
done
tries=${WAIT_FOR_S3_TRIES-60}
interval=${WAIT_FOR_S3_INTERVAL-1}
case $tries in
    '' | *[!0-9]* | 0)
        echo "$me: WAIT_FOR_S3_TRIES is a whole number of at least 1, not '$tries'" >&2
        exit 2
        ;;
esac

status=000
for try in $(seq 1 "$tries"); do
    # curl prints 000 when no answer came; its own exit code says no more.
    status=$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 \
        --aws-sigv4 aws:amz:us-east-1:s3 \
        --user "$VCON_TEST_S3_ACCESS_KEY:$VCON_TEST_S3_SECRET_KEY" \
        "${VCON_TEST_S3_ENDPOINT%/}/") || true
    if [ "$status" = 200 ]; then
        echo "$me: $name answered a signed request with 200 at try $try"
        exit 0
    fi
    if [ "$try" -lt "$tries" ]; then
        sleep "$interval"
    fi
done

if [ "$status" = 000 ]; then
    last="it gave no answer"
else
    last="its last answer was $status"
fi
echo "$me: $name never answered a signed request with 200 in $tries tries; $last" >&2
if [ -n "$log" ] && [ -f "$log" ]; then
    cat "$log" >&2
fi
exit 1
