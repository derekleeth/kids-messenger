# Setting up the family hub

The hub is a small program that keeps the family's messages, pictures and shared lists, and passes new messages to
every device as they arrive. It runs on one computer that stays on, usually the family PC. Nothing goes to any
outside service: the database and pictures are files in one folder.

## 1. Build and start it

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

| Option          | Default                     | What it does                                        |
| --------------- | --------------------------- | --------------------------------------------------- |
| `--data DIR`    | `./hub-data` (KITCHAT_DATA) | Where the database and pictures are kept. Back it up. |
| `--port N`      | `8787` (KITCHAT_PORT)       | The port the app connects to.                       |
| `--bind ADDR`   | `0.0.0.0`                   | Which network to listen on (`127.0.0.1` = this PC only, e.g. behind a tunnel). |
| `--no-previews` | previews on                 | Don't fetch link previews (titles and pictures for shared links). |

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
copied safely when nothing is being sent). It holds every message and picture.

## 5. Reaching it away from home (optional)

At home, the tablets talk to the hub over the home network. To also get messages on mobile data or at a friend's
house, pick one:

### Tailscale (simplest, private)

Install [Tailscale](https://tailscale.com) on the hub PC and on each phone or tablet, signed in to the same account.
Use the PC's Tailscale address as the hub address, for example `100.101.102.103:8787` or `family-pc:8787` with
MagicDNS. Only your own devices can reach the hub. Each device needs the Tailscale app running.

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
