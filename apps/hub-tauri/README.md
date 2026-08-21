# Amity Hub (`hub-tauri`)

The tablet "hub" — a SolidJS frontend in a Tauri v2 shell. It shows the calm
at-rest surfaces (Today, Week) and the Capture inbox, talking to `amity-service`
over loopback (`http://127.0.0.1:7890`).

This crate is **outside the top-level Cargo workspace** (its `src-tauri` has its
own `[workspace]` root) because Tauri needs system GUI libraries that aren't
guaranteed in CI. Build and run it separately, as below.

## Prerequisites

### System libraries (Ubuntu/Debian)

Tauri v2 on Linux needs WebKitGTK 4.1 and friends:

```sh
sudo apt update && sudo apt install -y \
  libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

(See the current Tauri docs for other distros.)

### Toolchain

- A recent stable Rust (the workspace pins `stable`).
- Node.js + npm (the frontend and the Tauri CLI). Install JS deps once:
  `cd apps/hub-tauri && npm install`.

## Run it

From the repo root, the one-command launcher starts the service and the hub
together:

```sh
scripts/run-hub.sh            # windowed
AMITY_KIOSK=1 scripts/run-hub.sh   # fullscreen / kiosk
```

Or manually: start `cargo run -p amity-service` (binds `127.0.0.1:7890`), then in
`apps/hub-tauri` run `npm run tauri dev`.

`npm run build` type-checks and builds the frontend only (no native window) — it
is what CI can run without the system GUI libraries.

## Notes

- **Tauri commands must not be `pub`.** Each `#[tauri::command]` fn in
  `src-tauri/src/lib.rs` is colocated with `tauri::generate_handler!`; marking one
  `pub` makes tauri-macros emit a re-export that collides with its generated
  helper (rustc E0255, "defined multiple times"). Keep them non-`pub`.
- Icons in `src-tauri/icons/` are placeholders — swap in a real Amity icon when
  there is one (regenerate with `npm run tauri icon <source.png>`).
