#!/usr/bin/env bash
# run-hub.sh — launch amity-service + the Tauri hub together for local use.
#
# The hub talks to amity-service over loopback and is hardcoded to
# http://127.0.0.1:7890 (SERVICE_BASE_URL in src-tauri/src/lib.rs), which is the
# service's own default bind/port. This script starts the service, waits for it
# to accept connections, then runs the hub via `tauri dev`. When the hub exits
# (or you Ctrl-C), the service this script started is stopped — by its exact PID,
# nothing else.
#
# Fullscreen/kiosk: set AMITY_KIOSK=1 to launch the hub fullscreen (read by the
# Tauri shell at startup). Default is windowed.
#
# Prerequisites (Ubuntu/Debian) — see apps/hub-tauri/README.md:
#   sudo apt install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
#     libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
set -euo pipefail

# Resolve the repo root from this script's location, then work from there.
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Build the service first so the readiness wait below times only its startup,
# not a cold compile.
echo "building amity-service ..."
cargo build -p amity-service

# Start the service in the background and capture its exact PID.
echo "starting amity-service on 127.0.0.1:7890 ..."
cargo run -p amity-service &
SERVICE_PID=$!

# Stop only the service we started, by its exact PID, whenever this script exits.
cleanup() { kill "$SERVICE_PID" 2>/dev/null || true; }
trap cleanup EXIT

# Wait up to ~30s for the service to accept TCP connections on 7890.
echo "waiting for the service to accept connections ..."
for _ in $(seq 1 60); do
  if (exec 3<>/dev/tcp/127.0.0.1/7890) 2>/dev/null; then
    exec 3>&- 3<&-
    break
  fi
  sleep 0.5
done

# Launch the hub in the foreground (its window opens on your display).
# AMITY_KIOSK is passed through for the shell's fullscreen check; default 0.
echo "launching hub (AMITY_KIOSK=${AMITY_KIOSK:-0}) ..."
cd apps/hub-tauri
AMITY_KIOSK="${AMITY_KIOSK:-0}" npm run tauri dev
