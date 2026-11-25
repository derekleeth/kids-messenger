# Plan: messaging without internet (camping and other places with no signal)

Status: **options to weigh.** Nothing here is built yet. The aim of this page is to choose a route.

Camping, a car with no signal, a big park, a ski hill: the family is fairly close together but no device can reach the
hub. Today the messages wait in each device's outbox until the hub is back. This page collects the ways KitChat could
still deliver messages in those places, what each costs, and how the hub fits in.

- [What it should do](#what-it-should-do)
- [What every route needs first](#what-every-route-needs-first)
- [The routes](#the-routes): [A. Same Wi-Fi](#a-same-wi-fi-on-the-travel-router-iroh) ·
  [B. Travel hub](#b-a-travel-hub-on-a-laptop-or-the-travel-router) ·
  [C. Meshtastic radios](#c-our-meshtastic-radios) ·
  [D. Phone-to-phone Bluetooth, Offline Protocol SDK](#d-phone-to-phone-bluetooth-and-wi-fi-direct-offline-protocol-sdk) ·
  [E. Other libraries](#e-other-libraries-looked-at)
- [Side by side](#side-by-side)
- [The hub: what it can do](#the-hub-what-it-can-do)
- [Licensing](#licensing)
- [Hardware we have](#hardware-we-have)
- [Suggested route](#suggested-route)
- [Open questions](#open-questions)

## What it should do

- Text, emoji, Kit stickers and reactions reach the family with no internet, and ideally with nothing to set up at the
  campsite.
- Only the family's own devices: a stranger's device is ignored.
- Kids use KitChat as always. No second app, no radio settings.
- A message shows how it went: _waiting_, _sent nearby_ (with a mark for Wi-Fi 📶 or radio 📡), then the usual tick once
  it's on the hub.
- Nothing is lost or doubled: when the hub is reachable again, the copies that came another way are replaced by the hub's
  copy with the same ID, in the right order.
- The hub keeps the one full history, so parents see everything that was said, whichever way it travelled.
- Little battery cost: extra radios and discovery are on only while the hub is out of reach, or while a parent turns on
  **Camping mode**.

Photos and videos, editing shared lists and the desktop app are welcome where a route makes them easy, but not required.

## What every route needs first

These are needed whichever route is chosen (except B, the travel hub, which needs none of them), and they help plain
offline sending too.

### 1. The family roster

Today the only proof of a device is its secret token, and only the hub can check it. Away from the hub, devices need to
know each other in advance:

1. Each device makes a key pair (Ed25519) and tells the hub its public key: `PUT /devices/me/peer { "key": "…" }`.
   For Meshtastic, a device also tells the hub the node ID of the radio it's paired with.
2. The hub adds these to each `Device` and sends the family's device list in `/sync` and a new `Event::Devices`, so every
   device keeps an up-to-date **roster**: key or node ID → device → member.
3. Anything from a key or node not in the roster is dropped.
4. A parent removing a device takes it out of the roster. A device that's offline at the time trusts it until its next
   sync; acceptable for a lost tablet, and the Devices screen should say so.

The roster comes from the hub over the connection the app already trusts (HTTPS, or a home address that passed
`/prove`), so there's no new trust step. Devices enrolled before this version join the roster at their next sync.

### 2. The time a message was written

The hub sets `created_at` when a message reaches it (`hub/src/routes/chats.rs`, `send`). A message written at 2 pm at the
lake and uploaded at 8 pm sorts at 8 pm. `SendMessage` gets an optional `sentAt`; the hub uses it if it's in the past,
no more than 7 days old, and matches the ULID's time, and otherwise uses the time it arrives.

### 3. One outbox, several ways out

A message typed with no hub goes into `outbox` as now. A new `src-tauri/src/nearby.rs` also hands it to whatever ways
are available (Wi-Fi peers, a radio), marks it _sent nearby_, and `flush` still sends it to the hub later with the same
ID. A received message goes into `messages` with `rev = 0` and a `via` mark; the hub's copy replaces it by ID through
`/sync` or `/live`. Each way out sits behind a small trait (`NearbyLink`), so routes can be added one at a time and the
tests can use a fake.

What goes over the air is a small versioned payload in a new `kit-protocol` module, shared by the app and the hub:

```rust
pub enum NearbyPayload {
    /// A message, with the same `id` and `body` it will have on the hub.
    Message { id: String, chat_id: String, sender_id: String, body: MessageBody, sent_at: i64 },
    React { message_id: String, emoji: String },
    Read { chat_id: String, message_id: String },
}
```

The receiver checks it like the hub does: the sender's key or node belongs to `sender_id`, `sender_id` is in `chat_id`,
the body is allowed on that route and within `MAX_TEXT_CHARS`, the sticker name is known, and the ULID's time is close
to `sent_at`.

**Size:** about 2–3 days, in the hub, `kit-protocol` and the app's sync. Useful before any route exists, because of
`sentAt`.

## The routes

### A. Same Wi-Fi on the travel router (iroh)

We take a travel router on trips. Every device joins its Wi-Fi, and the KitChat apps find each other on it and talk
directly, with no hub.

**How:** [iroh](https://github.com/n0-computer/iroh) (Rust, Apache-2.0, actively maintained). Each device's roster key is
its iroh ID. iroh finds the others on the local network (mDNS) and opens encrypted QUIC connections that check the key on
both ends, so a stranger on the same network can't pose as family. It's all on the Rust side, in `nearby.rs`, so it
works the same on Android and the Windows app. Android needs one small Kotlin addition: a Wi-Fi multicast lock while
it's looking for peers.

**The router must let devices talk to each other.** "AP isolation" / "client isolation" must be off. It usually is on
guest networks, so use the main network.

| Pros | Cons |
| --- | --- |
| Fast: text, stickers, reactions, and the chat-sized copy of photos | Only where the travel router (or some shared Wi-Fi) is |
| Android **and** desktop, from one Rust implementation | Everyone needs to be within Wi-Fi range of the router |
| Permissive licence (Apache-2.0), no licence decision | Phones have to stay on the router's Wi-Fi when it has no internet (Android may want to switch away; "keep using this network" prompt) |
| No new Android radio code | Range is the router's Wi-Fi, a campsite but not a hike |
| Fits the outbox and roster design directly | |

**Size:** about 1.5 weeks after the roster.

### B. A travel hub on a laptop or the travel router

Run `kitchat-hub` itself on the trip: on a laptop, or on the travel router if it runs OpenWrt (GL.iNet and similar) and
can take a hub built for its processor, with the data on a microSD card or USB stick. Each device gets the travel hub as
one more hub address; trying several hub addresses in order already exists.

| Pros | Cons |
| --- | --- |
| No new app code; **everything** works, photos, videos and lists included | Two hubs, two histories: messages from the trip have to be brought back into the home hub afterwards. A "sync this hub into the main one" tool is real work (IDs, `rev` numbers, members, media) |
| No licence questions | Needs the laptop or router running, and everyone in its Wi-Fi range |
| Combines well with C: a radio can be the travel hub's gateway | Running the hub on a router depends on its model, processor and storage |
| | The family has to be set up on the travel hub (or copied over before the trip) |

A variant without the merge: the travel hub is only a **cache** that passes messages on to the home hub whenever the
router has internet. That needs hub-to-hub forwarding, which is about the same work as the merge tool.

**Size:** none to start on a laptop with a copied family; 1–2 weeks for the merge or forwarding tool.

### C. Our Meshtastic radios

We have two SenseCAP T1000-E and a couple of Heltec boards running Meshtastic. The Meshtastic app is too complicated
for the kids, but the radios are made to be driven by other apps: the Meshtastic app is one client of an open device
API. KitChat would be another, and the kids never see Meshtastic.

**How:**

- **Tablet to radio.** Each kid's tablet pairs once with "its" T1000-E over Bluetooth LE, as the Meshtastic app does, and
  KitChat reconnects to it on its own in the background. A small Bluetooth client in Kotlin next to `KitPlugin.kt`
  passes bytes to and from Rust. The UI shows only "📡 Kit radio connected".
- **Hub to radio.** Over Wi-Fi (TCP port 4403, for ESP32 Heltecs) or a USB cable. The official Rust crate `meshtastic`
  (0.1.9) speaks USB serial, TCP and Bluetooth.
- **On the air.** LoRa carries about 200 bytes a packet, and a shared channel allows only a few messages a minute. Text,
  emoji, stickers and reactions fit; photos don't. KitChat sends a compact binary payload (ULID, chat, sender, text) on
  Meshtastic's private-app port, not as a Meshtastic text, so it doesn't clutter the Meshtastic app and KitChat knows
  its own messages.
- **Family only.** The radios are set up once on a private channel with a random key, so strangers' radios can't read
  or post. Anyone with the channel key could pose as another family radio; the roster maps each node ID to a person,
  and firmware 2.5 and later has per-node keys for direct messages that prove the sender.
- **Range:** kilometres, not metres. Different campsites, a hike, a ski hill.

**The hub as a LoRa gateway** (see [the hub section](#the-hub-what-it-can-do)): a Heltec on the hub's Wi-Fi or USB
passes radio messages into the hub as their senders, and hub messages out to members who are only reachable by radio.
We don't use Meshtastic's own MQTT internet bridge: it runs through outside servers.

| Pros | Cons |
| --- | --- |
| Range in kilometres; works on a hike, away from any Wi-Fi | Text, stickers and reactions only; slow, a few messages a minute |
| We already own the hardware | **GPL-3.0** (the Rust crate, the protobufs, the firmware): KitChat has to be released under the GPL, see [Licensing](#licensing) |
| The hub gateway reaches parents anywhere with internet | A radio takes one Bluetooth app at a time: set them up with the Meshtastic app once, then only KitChat connects |
| Cards are small, splash-proof, last days | Kids need the tablet **and** the card; the card has no screen to send on its own |
| Optional: the cards' GPS could show parents where the kids are | Pin a firmware version and test before updating the radios |
| | Bluetooth link between tablet and radio needs the background service to stay up |

**Size:** about 2 weeks for the tablet side (Bluetooth client, payload, setup screens), about 1 week for the hub gateway.

### D. Phone-to-phone Bluetooth and Wi-Fi Direct (Offline Protocol SDK)

The [Offline Protocol SDK](https://github.com/Offline-Protocol/offline-protocol-sdk) (0.34.0, 2026-11-19) gives
phone-to-phone messaging over Bluetooth LE and Wi-Fi Direct, with multi-hop relay, no router and no radio.

**What the SDK is:**

| | |
| --- | --- |
| Transports | Android: BLE, Wi-Fi Direct, LAN (mDNS `_offlineprotocol._tcp`), internet relay, Nostr, Reticulum (needs a gateway). Desktop: LAN only in practice: Windows can't be a BLE peripheral, and the desktop radio code exists only in the Python binding. |
| Engine | Pure Rust crate `offline-protocol`, but I/O-free: the radios are platform code. On Android they're Kotlin, in the AAR `com.offlineprotocol:offline-protocol-sdk`, marked "preview", with its own engine `.so`. |
| Identity | One Ed25519 key per install; its `off1…` address is derived from it and can't be faked. |
| Encryption | MLS (RFC 9420), on by default, fail-closed. |
| Messages | String content up to 1 MiB, delivery and failure events, outbox and retry for up to 7 days, multi-hop relay limited by battery, file transfer. |
| Trust | None built in: anyone in range can start a session. Family-only is the app's job (address allow-list from our roster, `block_user`, relay only for roster addresses). |
| Telemetry | Built in by default but inert until `enable_telemetry` is called; `default-features = false` leaves it out. |
| Android | Bluetooth scan/connect/advertise and Nearby Wi-Fi permissions; location on Android 12 and older; a `connectedDevice` foreground service; minSdk 24 (ours is 28). |
| Maturity | Weekly releases with frequent breaking changes (`docs/UPGRADING.md` is 3,300 lines), native Kotlin API "not decided". |
| Size | About 8–11 MB per architecture. |
| Licence | **AGPL-3.0-only or commercial.** |

**How it would fit:** the SDK runs on the Kotlin side (`NearbyBridge.kt` next to `KitPlugin.kt`) because the AAR already
has the Bluetooth, Wi-Fi Direct and foreground-service code; using the Rust crate directly would mean rewriting about
28,000 lines of radio glue from the SDK's FFI crate. `nearby.rs` keeps all of KitChat's logic. The roster carries each
device's `off1` address. The family chat is sent as one copy per device (a family is a handful of devices), which avoids
keeping MLS groups in step. `HubService` gets the `connectedDevice` type while it's on, and the radios turn on after 2
minutes without the hub or in Camping mode.

| Pros | Cons |
| --- | --- |
| Works with nothing but the phones: no router, no radio | Range is Bluetooth's, tens of metres (more with relay hops) |
| Multi-hop relay through other family devices | **AGPL-3.0** or a paid licence |
| End-to-end encrypted, keys can't be faked | Fast-moving SDK with breaking changes; "preview" Android API |
| Wi-Fi Direct could carry small photos | Background Bluetooth scanning costs battery, and some tablet makers stop background apps hard |
| | Android only in practice; no desktop |
| | 8–11 MB more per architecture |
| | Mostly overlaps with A (same Wi-Fi) and C (radios, longer range) |

**Size:** 2–3 day spike (does the AAR build alongside our `.so`; range and battery with the screen off), then about 2
weeks for text, 1 week for polish.

### E. Other libraries looked at

None of these gives Bluetooth and Wi-Fi together under a permissive licence as a drop-in library.

| Project | What | Licence | Why not (or when) |
| --- | --- | --- | --- |
| [rust-libp2p](https://github.com/libp2p/rust-libp2p) | Rust p2p networking, LAN discovery, no BLE | MIT | Works for A, but heavier and more general than iroh |
| [bitchat-android](https://github.com/permissionlesstech/bitchat-android) | Android BLE mesh chat app | GPL-3.0 | An app, not a library. A good model if we ever write our own Bluetooth mesh |
| [Berty / Wesh](https://github.com/berty/berty) | Go p2p stack for phones: BLE, mDNS, Android Nearby | Apache-2.0 / MIT | A Go runtime with its own message store inside our app: a lot to embed |
| [Briar / Bramble](https://github.com/briar/briar) | Java: Bluetooth, Wi-Fi, Tor | GPL-3.0 | Mature, but its own design, Java, GPL |
| Google Nearby Connections | Android: BLE, Wi-Fi Direct, hotspot | Free, closed | Needs Google Play services; KitChat uses no Google services, and Fire tablets lack them |
| Ditto | Commercial sync SDK over BLE, Wi-Fi, LAN | Paid | A paid outside service |
| Reticulum | Python mesh networking, LoRa and more | Own licence | Python; Meshtastic covers our radios already |
| Our own BLE | Android's Bluetooth LE advertising and GATT | Ours | Text between nearby tablets is a few hundred lines of Kotlin; multi-hop relay is where it gets hard. A fallback if D's licence or churn rules it out |

## Side by side

| | A. Same Wi-Fi | B. Travel hub | C. Meshtastic | D. Offline Protocol SDK |
| --- | --- | --- | --- | --- |
| Needs | Travel router | Laptop or router running the hub | Radios (owned) | Nothing |
| Range | Router's Wi-Fi | Router's Wi-Fi | Kilometres | Tens of metres, plus hops |
| Text, stickers, reactions | ✅ | ✅ | ✅ | ✅ |
| Photos | Chat-sized copies | ✅ everything | ❌ | Maybe, over Wi-Fi Direct |
| Lists and notes | Later | ✅ | ❌ | Later |
| Desktop app | ✅ | ✅ | Hub/USB only | ❌ |
| Licence | Apache-2.0 | None | GPL-3.0 | AGPL-3.0 or paid |
| New code | ~1.5 weeks | None, then 1–2 weeks to merge | ~3 weeks | ~3 weeks plus spike |
| Battery | Low | Low | Low (radio has its own) | Highest (BLE scanning) |
| Risk | Router isolation settings; Android leaving a no-internet Wi-Fi | Merging histories | GPL; firmware changes | AGPL; SDK churn; background limits |

## The hub: what it can do

Ranked by value for us:

1. **Roster and keys** (needed by A, C, D). Store each device's key and radio node ID, send the roster with `/sync`, take
   it out when a device is removed. Small changes to `devices`, `routes/members.rs` and `routes/sync.rs`.
2. **Correct times for late messages** (needed by all but B). Accept `sentAt`.
3. **LoRa gateway** (with C). A Heltec on the hub's Wi-Fi (TCP 4403) or USB. Radio messages go into the hub as their
   senders (by the roster's node IDs), so every phone with internet sees "on my way back 🐾" from a kid out of signal.
   Text and stickers from the app go out by radio to members reachable only that way. Works at home and, with the hub
   on the trip (B), at camp.
4. **Parent controls** (recommended). A family setting for who may use the offline routes (for example, kids only
   while Camping mode is on), and Camping mode for everyone switched on from the Family screen before a trip. Synced
   like members.
5. **One history for parents** (comes free). Every message still reaches the hub from its sender's outbox.
6. **Travel hub** (B), as a copy or a cache of the home hub.
7. **Carrying other people's messages to the hub** (later, optional). If a kid's tablet stays offline for days, the
   first family device back online could upload what it received on the sender's behalf. The hub would need the
   sender's signature with their roster key, which A can provide (the key is ours); D would need a signing call the
   SDK may not offer.
8. **Hub as an Offline Protocol node or relay** (not recommended). The hub could run an SDK node and hold encrypted
   messages for devices that are away, or we could host a Nostr relay. At home the hub is already reachable, and on the
   VPS it already covers "messages anywhere"; the SDK's own internet relay server isn't published.

## Licensing

The repository is public but has **no licence**: no `LICENSE` file, GitHub shows none, and `package.json` says
`"license": "UNLICENSED"`. Public code with no licence is "all rights reserved".

- **A and B** need no licence change (iroh is Apache-2.0).
- **C** (GPL-3.0 Meshtastic crate) means KitChat has to be released under the GPL, with its source offered to everyone
  who gets the app. Writing our own client from the published protocol docs might avoid that, but it's a grey area.
- **D** (AGPL-3.0 SDK) is the same, plus the AGPL's network clause, or a paid commercial licence.

If we go with C or D: add a `LICENSE` with **GPL-3.0-or-later** (it keeps D possible later, since GPL-3.0-or-later code
can be combined with AGPL-3.0 code), set `"license"` in `package.json` and `license` in the `Cargo.toml` files. Our other
dependencies (Tauri, React, axum, iroh) are MIT/Apache, which the GPL accepts. Kit's artwork (`art/kit/`) can be under a
separate licence (Creative Commons, or all rights reserved for the character), said in the README. Releases on GitHub
next to the tagged source already meet the "offer the source" requirement.

## Hardware we have

| Device | Chip / radio | Connects by | Best use |
| --- | --- | --- | --- |
| SenseCAP T1000-E (×2) | nRF52840, LR1110, GPS, IP65, no screen, no Wi-Fi | Bluetooth LE (or USB) | One per kid, paired with their tablet |
| Heltec (×2), model to check | If WiFi LoRa 32 V3/V4: ESP32-S3, SX1262, small screen, Wi-Fi + BLE. If T114: nRF52840, no Wi-Fi | ESP32: Wi-Fi/TCP **or** Bluetooth (not both at once), or USB. T114: Bluetooth or USB | One as the hub's gateway; one as a relay up high (tree, camper roof) in router role, or a parent's radio |

A possible setup:

| Device | Who | How KitChat reaches it |
| --- | --- | --- |
| T1000-E #1 | Kid 1, with their tablet | Bluetooth, from the Android app |
| T1000-E #2 | Kid 2, with their tablet | Bluetooth, from the Android app |
| Heltec #1 | Hub gateway, in the camper or at home | Wi-Fi/TCP from the hub, or USB |
| Heltec #2 | Relay up high, or a parent's radio | Nothing (relay), or Bluetooth from a parent's phone |

Parents without a radio get the kids' radio messages through the hub when they have internet, or over the travel
router's Wi-Fi (A).

**Radio setup checklist** (once, with the Meshtastic app, before KitChat takes over): same region and firmware version
on every radio; a private family channel with a random key; roles (CLIENT for the cards, ROUTER or CLIENT_MUTE for the
relay); Wi-Fi on the gateway Heltec; then forget them in the Meshtastic app so KitChat can connect.

**Ideas for later:** the cards' GPS sending kids' positions to parents only (off by default, in keeping with the app
taking location out of photos); a card's button sending a preset "I'm OK 👍" to parents through the gateway, if the
firmware allows it.

## Suggested route

1. **Roster and `sentAt`** (needed by every route but B; about 2–3 days).
2. **A. Same Wi-Fi with iroh** for trips with the travel router: Android and desktop, small photos, no licence change.
3. **C. Meshtastic radios and the hub gateway** for distance, once we've decided on the GPL.
4. **D. Phone-to-phone Bluetooth** probably isn't needed: A covers "close together" and C covers "far apart" better.
   Look at it again only if there are often trips with no router and no radios.
5. **B. Travel hub** in addition, if photos and lists on trips matter enough to build the merge tool.

## Open questions

1. **Licence:** release KitChat under GPL-3.0-or-later (needed for C or D)? Keep Kit's artwork under its own licence?
2. **Heltec model:** WiFi LoRa 32 V3/V4 (Wi-Fi gateway) or T114 (USB gateway)? The board name is printed on it.
3. **Travel router model:** can it run the hub (for B and a camp gateway)?
4. **Kids and the offline routes:** on for kids all the time, only in Camping mode, or only to reach a parent?
5. **Kids' location** from the cards' GPS: a parent-only map, or not at all?
6. **Relaying:** may kids' devices pass on others' messages (range against battery), or parents' only?
