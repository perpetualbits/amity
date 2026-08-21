#!/usr/bin/env bash
# seed-demo.sh — populate a RUNNING amity-service (127.0.0.1:7890) with demo data
# for the CURRENT week, so the hub's Today and Week views have representative
# content to look at: all-day + timed events, a rescheduled event, an annotated
# event, and dated tasks. Re-running just adds more copies.
#
# Start the service first (scripts/run-hub.sh does that), then run this once in
# another terminal. Requires curl and python3 (both standard).
set -euo pipefail
BASE="http://127.0.0.1:7890/api/v1"

# Days of the current week (local time), Monday-anchored. `date +%u` is the ISO
# weekday (1=Mon..7=Sun), so this week's Monday is today minus (u-1) days.
# (Avoid `date -d "monday this week"` — GNU date resolves it to NEXT Monday.)
MON=$(date -d "$(( $(date +%u) - 1 )) days ago" +%Y-%m-%d)
TUE=$(date -d "$MON +1 day" +%Y-%m-%d)
WED=$(date -d "$MON +2 day" +%Y-%m-%d)
THU=$(date -d "$MON +3 day" +%Y-%m-%d)
FRI=$(date -d "$MON +4 day" +%Y-%m-%d)
TODAY=$(date +%Y-%m-%d)

# POST JSON to an endpoint; extract the "id" field from a JSON response.
# `--fail` makes an HTTP 4xx/5xx a non-zero exit, so (with `set -e`) a rejected
# payload aborts loudly instead of the script reporting "done" with fewer items.
post() { curl -fsS -X POST "$BASE/$1" -H 'content-type: application/json' -d "$2"; }
idof() { python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])'; }

echo "seeding demo data for the week of $MON ..."

# Dated tasks — surface on their due day (Today and Week).
post tasks "{\"title\":\"Water the plants\",\"due_by\":\"${TODAY}T18:00:00Z\",\"effort\":1}" >/dev/null
post tasks "{\"title\":\"Return library books\",\"due_by\":\"${THU}T17:00:00Z\",\"effort\":2}" >/dev/null

# Events — an all-day one (leads its day) and timed ones across the week.
post events "{\"title\":\"School closed (staff day)\",\"start_at\":\"${WED}T00:00:00Z\",\"all_day\":true}" >/dev/null
post events "{\"title\":\"Dentist\",\"start_at\":\"${TUE}T09:00:00Z\"}" >/dev/null
post events "{\"title\":\"Football practice\",\"start_at\":\"${WED}T18:00:00Z\"}" >/dev/null

# An event RESCHEDULED via an override (Thu 15:00 → 17:00): shows the moved marker.
RESCHED=$(post events "{\"title\":\"Piano lesson\",\"start_at\":\"${THU}T15:00:00Z\"}" | idof)
post "events/${RESCHED}/override" "{\"instance_date\":\"${THU}\",\"action\":\"reschedule\",\"payload\":\"${THU}T17:00:00Z\"}" >/dev/null

# An event ANNOTATED via an override (Fri 10:00): shows the note.
ANNOT=$(post events "{\"title\":\"Parent-teacher meeting\",\"start_at\":\"${FRI}T10:00:00Z\"}" | idof)
post "events/${ANNOT}/override" "{\"instance_date\":\"${FRI}\",\"action\":\"annotate\",\"payload\":\"Bring last term's report\"}" >/dev/null

echo "done — open the hub; Today and Week should now show the seeded items."
echo "(Note: an external recurring feed with an EXDATE needs a real ICS URL and is"
echo " not seeded here — the reschedule/annotate/all-day/timed/task cases are.)"
