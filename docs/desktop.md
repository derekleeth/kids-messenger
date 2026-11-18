# KitChat on a laptop or desktop (Windows)

The desktop app is the same KitChat as on the tablets, laid out for a big screen: chats on the left, the open chat in
the middle and, with **Links & lists**, everything that was sent in it on the right.

## Install it

Download `KitChat_<version>_x64-setup.exe` from the repository's GitHub **Releases** ([releasing.md](releasing.md)), or
build the installer yourself:

```bash
npm install
npm run desktop:build
```

The installer is then `target/release/bundle/nsis/KitChat_<version>_x64-setup.exe`.

It installs for your Windows user only (no admin needed). The installer isn't signed, so the first time Windows
SmartScreen says it "protected your PC": choose **More info › Run anyway**.

Open it, choose **Join my family**, and type the hub address and a join code. A parent makes the code on a device
that's already in the family: **Family**, then the QR icon next to the person (or **Add another device for me**).

## Living in the tray

- Closing the window doesn't quit KitChat: it keeps running in the tray (by the clock), so messages still arrive
  and show a notification. Click Kit in the tray to open it again; right-click for **Quit KitChat**.
- **Start when I sign in** (in the tray menu, or **Family › This device**) starts KitChat in the tray when you sign in
  to Windows.
- The number of unread messages shows on KitChat's taskbar button and in the tray's tooltip.
- After a notification, bringing KitChat forward (clicking the notification, the tray icon or the taskbar button)
  opens that chat.

## Links & lists

The link icon at the top right of a chat opens everything sent in it that's worth getting back to:

- **Links**: every web link, newest first, with the site, who sent it and when. Click to open it in your browser, copy
  one, or **Copy all** (one per line, with titles, handy for pasting into a document).
- **Lists**: every list or note shared in the chat. Click to open it.

The search box finds words in the link, its title or the message it came with. It covers the messages this device
has; scroll further back in the chat to bring in older ones.

## Hub addresses

**Family › This device › Family hub › Change** holds the addresses KitChat tries for the family hub, from the top. Use
it if the hub moves (for example from the home PC to a server, see [vps_setup.md](vps_setup.md)), or to add a second
address ([hub_setup.md](hub_setup.md#tailscale-with-the-home-network-as-backup)). The device stays signed in.
