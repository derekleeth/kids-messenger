# Setting up the family hub

The hub is a small program that keeps the family's messages, pictures and shared lists, and passes new messages to
every device as they arrive. It runs on one computer that stays on, usually the family PC. Nothing goes to any
outside service: the database and pictures are files in one folder.

## 1. Get it and start it

Download `kitchat-hub_<version>_windows-x64.exe` from the repository's GitHub **Releases** ([releasing.md](releasing.md))
and rename it `kitchat-hub.exe`, or build it yourself:

```bash
cargo build -p kitchat-hub --release
```

This makes `target/release/kitchat-hub.exe`. Copy it somewhere permanent, for example `C:\KitChat\`, and start it:

```bash
C:\KitChat\kitchat-hub.exe --data C:\KitChat\hub-data
```

On its first start it prints a **setup code** (also saved in `hub-data\SETUP-CODE.txt`):

```
  ┌──────────────────────────────────────────────┐
  │  KitChat hub is new. Setup code:  4K9TQX      │
  │  Type it into the app to set up your family.  │
  └──────────────────────────────────────────────┘
```

Options:

| Option                 | Default                       | What it does |
| ---------------------- | ----------------------------- | ------------ |
| `--data DIR`           | `./hub-data` (KITCHAT_DATA)   | Where the database and pictures are kept. Back it up. |
| `--port N`             | `8787` (KITCHAT_PORT)         | The port the app connects to. |
| `--bind ADDR`          | `0.0.0.0` (KITCHAT_BIND)      | Which network to listen on (`127.0.0.1` = this computer only, e.g. behind a tunnel or nginx). |
| `--no-previews`        | previews on (KITCHAT_NO_PREVIEWS=1 turns them off) | Don't fetch link previews (titles and pictures for links in chats and lists). |
| `--trust-proxy`        | off (KITCHAT_TRUST_PROXY=1)   | The hub is behind a reverse proxy on the same computer (nginx): take each device's address from `X-Real-IP` (or the last `X-Forwarded-For`), but only from connections that come from `127.0.0.1`/`::1`. Use it with `--bind 127.0.0.1`; the hub warns if you don't. |
| `--rate-limit-scale X` | `1` (KITCHAT_RATE_LIMIT_SCALE) | Multiplies the rate limits below, e.g. `2` for twice as many requests. |

A flag wins over its environment variable.

The hub protects itself on its own: joining and setting up (`/hello`, `/setup`, `/enroll`) are limited to 10 a minute
per address; an address that sends 20 wrong device tokens in 10 minutes is refused for 15 minutes; each device may make
20 requests a second, each address 40; an address gets 5 wrong codes in 10 minutes (and the whole hub 10); each device
may keep 3 live connections open, 64 in all. Requests that run over 30 seconds are stopped (except live connections
and pictures and videos). Too many requests get `429 Too Many Requests` with a `Retry-After` time; the app waits and
tries again.

Link previews are fetched by the hub, not the kids' tablets, and only from public internet addresses.

## 2. Let the tablets reach it at home

Windows asks whether to allow the hub through the firewall the first time it runs. Allow it on **private** networks.
Or, in an admin PowerShell:

```powershell
New-NetFirewallRule -DisplayName "KitChat hub" -Direction Inbound -Protocol TCP -LocalPort 8787 -Profile Private -Action Allow
```

Find the PC's address with `ipconfig` (for example `192.168.1.20`). In the app, the hub address is then
`192.168.1.20:8787`. Give the PC a fixed address in your router (a "DHCP reservation") so it doesn't change.

## 3. Set up the family

1. Install the desktop app on the PC (`npm run tauri build`; the installer is in
   `target/release/bundle/nsis/`) or the APK on your phone ([sideload.md](sideload.md)).
2. Open it, choose **Set up a new family**, and type the hub address, the setup code, a family name and your name.
   You're the first parent. The setup code then stops working.
3. In **Family › Add someone**, add the other parent and the kids.
4. For each device: tap the QR icon next to the person, and on their tablet choose **Join my family** and scan the
   code (or type the 6 letters and the hub address). Codes work once, for 10 minutes.

A parent can remove a lost or old device under **Family › Devices**; it's signed out at once.

## 4. Keep it running

The hub should start with the PC. With Task Scheduler (admin PowerShell, adjust the paths):

```powershell
$action  = New-ScheduledTaskAction -Execute "C:\KitChat\kitchat-hub.exe" -Argument "--data C:\KitChat\hub-data" -WorkingDirectory "C:\KitChat"
$trigger = New-ScheduledTaskTrigger -AtStartup
$settings = New-ScheduledTaskSettingsSet -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) -ExecutionTimeLimit 0
Register-ScheduledTask -TaskName "KitChat hub" -Action $action -Trigger $trigger -Settings $settings -User "SYSTEM" -RunLevel Highest
```

Also turn off sleep for that PC when plugged in (Settings › System › Power), or messages wait until it wakes up.

**Back up** the `hub-data` folder now and then (copy it while the hub is stopped, or any time: SQLite's files can be
copied safely when nothing is being sent). It holds every message, photo and video. Full-size photos and videos add
up: a minute of phone video is around 100-200 MB, so keep an eye on the drive it's on.

## 5. Reaching it away from home (optional)

At home, the tablets talk to the hub over the home network. To also get messages on mobile data or at a friend's
house, pick one:

### Your own server (VPS)

If you rent a Linux server and have a domain, the hub can live there instead of on the family PC, behind nginx with
an `https://` address. That's a longer setup, so it has its own guide: [vps_setup.md](vps_setup.md). It also covers
moving this PC's `hub-data` across without anyone re-joining.

### Tailscale (simplest, private)

Install [Tailscale](https://tailscale.com) on the hub PC and on each phone or tablet, signed in to the same account.
Use the PC's Tailscale address as the hub address, for example `100.101.102.103:8787` or `family-pc:8787` with
MagicDNS. Only your own devices can reach the hub. Each device needs the Tailscale app running.

### Tailscale, with the home network as backup

A device can keep several addresses for the hub. It tries them from the top and stays on the first that works until
that connection drops. With the Tailscale address first and the home address second, the family can still chat at home
when the internet is down, as long as the hub PC and the Wi-Fi are on. Away from home it still takes the hub PC, its
internet connection and Tailscale.

1. Update the hub first: a device won't use several addresses with an older hub.
2. The hub already listens on all of the PC's networks, unless you changed `--bind`. Give the PC a fixed home address
   (a DHCP reservation on your router), so the saved one keeps working.
3. On each device, open **Family › This device › Family hub › Change**. Put the Tailscale address first (for example
   `100.101.102.103:8787`), tap **Add another address**, and add the home one (for example `192.168.1.20:8787`).

At least one address has to work when you save. One that can't be reached from where you are is kept, and KitChat says
so. A join code made on a device with several addresses carries them all when it's scanned, so new devices get the
list. Typing a code by hand gives the new device just the address you typed.

Home and Tailscale addresses are plain `http`, and the same home address can belong to a stranger's device on another
network. So a device with several addresses first asks the hub to prove it's yours, and sends its sign-in token only
to a hub that can. The hub answers that only for devices on the home network or Tailscale.

### Cloudflare Tunnel (no app on the tablets)

Gives the hub an `https://` address on the internet without opening a port on your router. You need a domain on
Cloudflare (free plan is fine).

```bash
cloudflared tunnel login
cloudflared tunnel create kitchat
cloudflared tunnel route dns kitchat kitchat.yourdomain.com
cloudflared tunnel run --url http://localhost:8787 kitchat
```

Install it as a service with `cloudflared service install` (see Cloudflare's docs) so it starts with the PC, and
start the hub with `--bind 127.0.0.1` so it's only reachable through the tunnel. The hub address in the app is then
`kitchat.yourdomain.com` (the app uses https for anything that isn't a home-network address, and refuses plain http
outside the home network).

Every request to the hub needs a device token that only enrolled devices have, so a public address doesn't let
anyone in. Join codes are locked after 10 wrong guesses.

## Trouble?

- **"Can't reach the family hub"**: is the hub running? Is the PC awake? Same Wi-Fi? Try `http://<address>:8787/hello`
  in a browser on the tablet: it should show `{"app":"kitchat-hub",...}`.
- **Android 17 tablets**: the first time, allow "devices on your local network" when asked.
- Logs: the hub prints to its console; run it with `RUST_LOG=debug` for more.
