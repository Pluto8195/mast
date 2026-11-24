#!/bin/sh
# Only synthetic fixture PRs are supported. No command reaches the network.
printf '%s\n' gh >> "${MAST_DRIVER_STUB_LOG:?}"
emit_pr() {
number=$1
case $((number % 3)) in
    0) state=MERGED; merged='"2026-09-09T00:00:00Z"' ;;
    1) state=OPEN; merged=null ;;
    2) state=CLOSED; merged=null ;;
esac
if [ $((number % 6)) -eq 1 ]; then
    checks='[{"status":"COMPLETED","conclusion":"SUCCESS"}]'
else
    checks='[{"status":"IN_PROGRESS","conclusion":null}]'
fi
printf '{"number":%s,"url":"https://github.com/fixture/repository/pull/%s","state":"%s","isDraft":false,"reviewDecision":"APPROVED","statusCheckRollup":%s,"reviews":[],"title":"Fixture pull request","headRefName":"fixture","headRefOid":"0123456789012345678901234567890123456789","mergeable":"MERGEABLE","reviewRequests":[],"author":{"login":"fixture"},"mergedAt":%s,"additions":1,"deletions":0}\n' "$number" "$number" "$state" "$checks" "$merged"
}
if [ "$1" = driver-fixtures ]; then
    case "$2" in *[!0-9]*|'') exit 127 ;; esac
    [ "$2" -le 100000 ] || exit 127
    index=1
    while [ "$index" -le "$2" ]; do
        emit_pr "$index"
        index=$((index + 1))
    done
    exit 0
fi
if [ "$1" != pr ] || [ "$2" != view ]; then
    echo 'driver: command disabled' >&2
    exit 127
fi
case "$3" in
    https://github.com/fixture/repository/pull/*) number=${3##*/} ;;
    *[!0-9]*|'') echo 'driver: command disabled' >&2; exit 127 ;;
    *)
        number=$3
        [ "$4" = --repo ] && [ "$5" = fixture/repository ] || exit 127
        ;;
esac
case "$number" in *[!0-9]*|'') exit 127 ;; esac
emit_pr "$number"
