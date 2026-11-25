# Plan: several hubs that keep each other in step

Status: **phases 1–3 done** (pairing and keeping in step, from the command line); phase 4 next. Part of route B in [nearby_messaging_plan.md](nearby_messaging_plan.md).

Today a family has one hub, at home or on a VPS. On a trip with no internet, a second hub on the travel router (or a
laptop) would let everyone keep chatting, with photos and lists, over the router's Wi-Fi. Copying the home hub's data
onto it works for a try, but the two copies then drift apart, and each device's sync position gets confused between
them. This plan lets a family have **several hubs that keep each other in step**: each has a full copy of the family,
and they swap changes whenever they can reach each other. A device can use any of them.

## Why it fits KitChat

- **IDs are already unique everywhere.** Messages, notes, members, devices and media are ULIDs, so two hubs never give
  the same ID to two different things.
- **Most of what's stored only grows.** A message's body never changes after it's sent; swapping messages is "add the
  ones I don't have".
- **Lists already merge.** `notes::merge` (`crates/kit-protocol/src/notes.rs`) combines two edits block by block, the
  newest edit of each, with markers for deleted blocks. That's what two hubs that both edited a list need.
- **Every change is numbered.** Each change already takes the hub's next change number (`rev`), so "everything since
  change N" is a query per table.

## How it works

### Hubs know each other

Each hub has its own **hub ID**, made the first time it starts, and every hub of a family shares the **family ID**,
made when the family is set up. Both are in `/hello` and `/sync`. Hubs pair only if their family IDs match.

Pairing: a parent gets a hub code, good once for 10 minutes: for now with `kitchat-hub pair-code` on the home hub
(`POST /hubs/codes`; phase 5 adds **Family › Hubs › Add a travel hub** to the app). On the router, with the hub
stopped:

```bash
kitchat-hub --data /tmp/mountd/disk1_part1/kitchat/hub-data join https://kitchat.example.com CODE --name "Beryl"
```

It downloads the whole family from the home hub (a new travel hub needs no setup of its own), and the two share a
secret for their later syncs: home keeps only its hash; the travel hub keeps it as it is, as it has to send it
(`Authorization: Hub <hub ID>:<secret>`). A hub that already has a different family refuses to join, without spending
the code. `kitchat-hub sync` swaps changes once by hand; `GET /hubs` lists the paired hubs and `DELETE /hubs/{id}`
unpairs one (parents only).

### The travel hub calls home

The router sits behind the campground's (or phone hotspot's) network, so the home hub can't reach it: the travel hub
always starts the connection. In one session it:

1. asks home for every change since the last one it has seen from home, and applies them;
2. sends home its own changes since the last one home has seen from it.

The travel hub keeps how far it has got each way, so a dropped connection carries on, and taking in the same changes
twice does nothing. While connected, it keeps a request for home's changes open (`GET /hubs/changes?wait=true`;
home answers as soon as it has something, or after 20 seconds), so changes from home arrive within moments, and it
sends its own about 0.3 seconds after they happen (and at least once a minute). If home can't be reached it tries
again after 2 seconds, then longer, up to a minute. Changes that arrive from the other hub are passed to the devices
connected to this one over `/live`, as if they'd been made here.

An unencrypted (`http`) address, like `192.168.1.20:8787` at home, could be a stranger's machine on another
network, so before sending anything there the travel hub asks it to show it knows the secret (`/hubs/prove`, which home
answers only on the home network or Tailscale), as devices check their hub. Plain `http` to a public address is
refused.

### Merge rules

Applying the same change twice, or two changes in either order, gives the same result; and a change that alters
nothing takes no new change number, so a hub sending back what it was just sent doesn't start a loop.

| What | Rule |
| --- | --- |
| Messages | Added if missing. If two hubs have different times for one message, both keep the earlier. A link preview is added if missing. |
| Reactions | Newest wins, per member and message. A removed reaction is kept as an empty one with its time, so it can't come back. |
| "Seen" (`read_up_to`) | The later message wins (ULIDs sort by time). |
| Lists and notes | `notes::merge`; a deleted list stays deleted. |
| Members (name, colour, picture) | Newest edit wins (`updated_at`). |
| Chat members | Combined. |
| Devices | Combined. **Removed stays removed**: a device removed on either hub is removed on all. `last_seen` is the later. |
| 1:1 chats | Their ID is worked out from the two member IDs, so two hubs that each made the same pair's chat while apart made the same chat. |
| Photos and videos | The record is copied straight away. The file is copied in the background, newest first, up to a size limit; a hub without a file fetches it from the other when someone opens it. |
| Family name | Newest edit wins. |
| Join codes, uploads in progress, rate limits | Stay on the hub they're on. |

Times in these rules come from the devices where possible (a message's `sentAt`, a note block's `updatedAt`), not from
the hub's clock: the router has no battery-backed clock and can be wrong after a restart without internet.

### Devices

- **A sync position per hub.** A device keeps the change number it has reached separately for each hub ID, so going
  router → home → router never skips anything. A hub it hasn't used yet gets an ordinary first sync. A device updated
  from before this keeps its one position for the first hub that tells it an ID (which is the hub it was using), and an
  old hub without an ID goes on using that single position.
- **Hub addresses arrive with the sync.** The family's hubs and their addresses are sent with `/sync`, so the router's
  address is added on every device without anyone typing it.
- **Any hub will do.** Because the hubs are in step, some devices on the home hub and some on the router is fine
  whenever the router has internet.

## Things to know

- **A device removed while hubs are apart** keeps working on the hub that hasn't heard yet, until they next swap
  changes. Fine for a lost tablet.
- **Trust.** A paired hub is trusted completely: it has every message and the hashes of every device's token. Pairing
  takes a code from a parent. Home stores the shared secret only as a hash; the travel hub has to keep it as it is, so
  its data folder should be guarded like home's (it has the whole family anyway).
- **Space.** The router doesn't need every old video: it copies every record but files only up to a limit (say the
  last 30 days or 20 GB), and fetches older ones from home when opened, if there's internet.
- **Only the travel hub calls.** Home lists the travel hub (`GET /hubs`) but never calls it, and keeps no positions
  for it; a home hub on a VPS or behind Tailscale works the same.
- **Changes go back once.** Rows the travel hub takes in from home take its own change numbers, so it sends them back;
  home finds nothing new in them and changes nothing. A little extra traffic, no harm; it could be skipped later.
- **Two separate families can't be joined** into one: their family IDs differ, so they refuse to pair.

## Running the hub on the Beryl 7

The GL.iNet GL-MT3600BE Beryl 7 has a MediaTek MT7987A (4 × ARM Cortex-A53, 2.0 GHz, 64-bit), 512 MB of memory, 512 MB
of flash (mostly the firmware), a USB 3.0 port, and GL.iNet's firmware built on OpenWrt, with SSH.

1. **Build** the hub for 64-bit ARM. Like the Linux hub in `.github/workflows/release.yml`, it's a static musl build;
   SQLite and `ring` have C in them, so cross-compiling is easiest with `cargo-zigbuild`:

   ```bash
   rustup target add aarch64-unknown-linux-musl
   cargo install cargo-zigbuild        # also needs Zig
   cargo zigbuild -p kitchat-hub --release --target aarch64-unknown-linux-musl
   ```

   Phase 6 adds this to the release workflow as `kitchat-hub_<version>_linux-aarch64.tar.gz`.
2. **Storage:** a USB stick or small SSD, ext4 if possible. GL.iNet mounts it under `/tmp/mountd/disk1_part1`
   (check with `df -h`).
3. **Copy and start** (SSH as `root` at `192.168.8.1`, the admin password):

   ```bash
   scp -O kitchat-hub root@192.168.8.1:/tmp/mountd/disk1_part1/kitchat/
   ssh root@192.168.8.1
   cd /tmp/mountd/disk1_part1/kitchat && chmod +x kitchat-hub
   ./kitchat-hub --data ./hub-data --port 8787
   ```

4. **Join the family** once, before starting it for good: `kitchat-hub pair-code` on the home hub, then on the router
   `./kitchat-hub --data ./hub-data join https://kitchat.example.com CODE --name Beryl` (see
   [Hubs know each other](#hubs-know-each-other)). The running hub then keeps in step with home whenever the router
   has internet.
5. **Start with the router:** a procd script `/etc/init.d/kitchat-hub` that waits for the USB drive and restarts the
   hub if it stops; `/etc/init.d/kitchat-hub enable`; and the script listed in `/etc/sysupgrade.conf` so firmware
   updates keep it.
6. **Devices** reach it at `192.168.8.1:8787`. The router's LAN lets devices reach it by default, and its addresses are
   private, so the hub's "prove you're ours" check works as at home.

## Phases

| Phase | What | Size | Status |
| --- | --- | --- | --- |
| 1. Groundwork | Hub ID and family ID in `/hello` and `/sync`; the app keeping a sync position per hub; `sentAt` on messages; reaction times (and removed reactions kept as empty); `updated_at` on members; 1:1 chat IDs from the member pair; change numbers on devices and media. Useful by itself, and fixes the sync-position problem of a copied hub. | 3–4 days | Built (PR #7) |
| 2. Swapping changes | A hub-only "every change since N" endpoint, applying it with the rules above, and a test that runs two hubs side by side: write on both, swap, check they match, swap again, check nothing changes. | ~1 week | Built (PR #8) |
| 3. Pairing and the sync loop | Hub codes, `kitchat-hub join`, the table of paired hubs, the call-home loop, and passing changes from the other hub to connected devices. | 3–4 days | Built (PR #9) |
| 4. Photos and videos | Copying files in the background with a size limit, and fetching from the other hub when opened. | ~3 days | Next |
| 5. App | **Family › Hubs** (each hub, last swapped, add a travel hub); hub addresses arriving with the sync. | 2–3 days | To do |
| 6. Router | The ARM build in the release workflow, and a `docs/travel_hub.md` for the Beryl 7. | 1 day | To do |

After phases 1–3, text, stickers, reactions and lists work across hubs; photos come with phase 4.
