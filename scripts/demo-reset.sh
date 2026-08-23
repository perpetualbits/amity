#!/usr/bin/env bash
# demo-reset.sh — reset the local Amity database to a clean, curated demo set.
#
# Backs up the current database (non-destructively, to a timestamped .bak),
# creates a fresh one, and seeds ONE coherent week: dinners (including tonight's),
# a couple of pantry staples, and a few tasks/events. The grocery list is left
# EMPTY on purpose so you can hit "Generate from this week's menu" live during the
# demo (the pantry staples get suppressed — a nice beat).
#
# When it finishes, launch the hub:
#     ./scripts/run-hub.sh
#
# IMPORTANT: stop the hub first. The running service holds the database open, so
# this script refuses to run while port 7890 is in use. Your previous data is
# preserved in the .bak file — restore it by moving it back over amity.db.
set -euo pipefail

# Work from the repo root (this script lives in scripts/).
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Refuse to run while the service is up — it holds the DB open, and resetting it
# underneath a live service would corrupt the demo. Tell the user to stop first.
if (exec 3<>/dev/tcp/127.0.0.1/7890) 2>/dev/null; then
  exec 3>&- 3<&-
  echo "error: something is already listening on 127.0.0.1:7890 (the hub is running)." >&2
  echo "       Stop the hub (Ctrl-C in its terminal), then re-run this script." >&2
  exit 1
fi

# Resolve the database path exactly as the service does (XDG-aware on Linux).
DB="${XDG_DATA_HOME:-$HOME/.local/share}/amity/amity.db"

# Back up any existing database to a timestamped file, then remove the original
# so the service recreates a fresh, empty one (migrations run on startup).
if [ -f "$DB" ]; then
  BAK="$DB.bak.$(date +%Y%m%d-%H%M%S)"
  mv "$DB" "$BAK"
  echo "backed up existing database → $BAK"
fi

# Build the service so the readiness wait below only times its startup.
echo "building amity-service ..."
cargo build -p amity-service

# Start the service in its own process group so we can stop exactly it (and any
# child) on exit — never anything else. Capture its PID.
echo "starting a temporary service to seed the fresh database ..."
setsid ./target/debug/amity-service >/tmp/amity-demo-reset.log 2>&1 &
SERVICE_PGID=$!

# Stop only the service group we started, whenever this script exits.
cleanup() { kill -TERM -- -"$SERVICE_PGID" 2>/dev/null || kill "$SERVICE_PGID" 2>/dev/null || true; }
trap cleanup EXIT

# Wait up to ~30s for the service to accept connections on 7890.
for _ in $(seq 1 60); do
  if (exec 3<>/dev/tcp/127.0.0.1/7890) 2>/dev/null; then exec 3>&- 3<&-; break; fi
  sleep 0.5
done

# Seed the standard week (tasks, events, meals, pantry staples), then add a
# dinner for TODAY so "tonight's dinner" shows on the Today view during the demo.
echo "seeding demo data ..."
bash scripts/seed-demo.sh >/dev/null
curl -fsS -X POST "http://127.0.0.1:7890/api/v1/meals" \
  -H 'content-type: application/json' \
  -d "{\"name\":\"Homemade pizza\",\"date\":\"$(date +%F)\",\"slot\":\"dinner\",\"ingredient_lines\":[{\"name\":\"pizza dough\"},{\"name\":\"mozzarella\"},{\"name\":\"passata\"},{\"name\":\"basil\"}]}" \
  >/dev/null
echo "added tonight's dinner (Homemade pizza)."

# Quick sanity check so a failed seed doesn't surprise you mid-demo.
MEALS=$(curl -fsS "http://127.0.0.1:7890/api/v1/meals" | python3 -c 'import sys,json;print(len(json.load(sys.stdin)))')
echo "seeded $MEALS meals for this week."

echo
echo "clean demo data ready. Now launch the hub:"
echo "    ./scripts/run-hub.sh"
echo "(then in the demo: Today → Week → Menu → Groceries → \"Generate from this week's menu\" → tap to check off)"
