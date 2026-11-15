# KitChat 🐾

A private, bright and friendly messenger for our family, with Kit the baby lynx. The kids can message us, send
pictures and screenshots, share links and make lists, all with only the family, and without any outside service.

- **Chats**: one family group chat and a 1:1 chat with each person. Text, emoji, Kit stickers, reactions, "seen".
- **Photos, videos and screenshots** at full size: from the photo picker, pasted, dragged in, or shared from any app
  with Android's Share button. The chat shows a small copy; tapping opens the full photo or plays the video. Big files
  upload in 4 MB chunks and carry on where they stopped after a dropped connection (up to 1 GB each). Location data
  (photo EXIF, a video's GPS boxes) is removed on the device and again on the hub.
- **Links**: shown as cards with a title and picture (fetched by the hub, not the tablet), and always opened in the
  browser.
- **Lists and notes**: checklists, cool sites, big ideas. Private by default; send one to a chat or make it a
  family list everyone can tick off.
- **Closed family**: nobody can join without a code from a parent; parents can remove a device at any time.
- **Works offline**: messages wait in an outbox and send when the hub is reachable again.

## How it fits together

```
 tablets / phones (Android)      desktop (Windows)
        KitChat app ─────┬────── KitChat app
                         │  https or home Wi-Fi
                  family hub (kitchat-hub)
                  on the family PC: SQLite + pictures
```

| Folder                 | What                                                                       |
| ---------------------- | -------------------------------------------------------------------------- |
| `src/`                 | The app's screens (React, TypeScript, Tailwind)                            |
| `src-tauri/`           | The app's Rust side: local cache, outbox, hub connection, notifications    |
| `src-tauri/gen/android`| Android: share target, background service, QR scanner (`KitPlugin.kt`)     |
| `hub/`                 | The family hub server (axum, SQLite)                                       |
| `crates/kit-protocol/` | What the app and the hub send each other, shared by both                   |
| `art/kit/`             | Kit's original artwork; `node scripts/kit-art.mjs` makes the app's PNGs    |

Same stack as TerminalLynx: Tauri 2, Rust (edition 2024), React 19, Vite, Tailwind 4.

## Getting started

- [docs/hub_setup.md](docs/hub_setup.md): run the hub and set up the family.
- [docs/sideload.md](docs/sideload.md): build the Android APK and install it on the tablets.

## Developing

```bash
npm install
npm run dev                # the UI in a browser with a pretend family (no hub needed); ?as=kid, ?welcome
npm run hub                # a hub on :8787 (data in ./hub-data)
npm run tauri dev          # the desktop app
npm run tauri android dev  # on a connected device or emulator
```

Two copies of the desktop app on one PC (to chat with yourself): start the second with `KITCHAT_PROFILE=kid`.

Checks:

```bash
npm run typecheck && npm run lint && npm test
cargo test --workspace && cargo clippy --workspace --all-targets
```
