# Running the hub on a VPS

This puts the family hub on a rented Linux server (a VPS) with your own domain, so the family gets messages
anywhere: at home, on mobile data, at a friend's house. It assumes:

- an **Ubuntu** VPS (22.04 or 24.04) that you can SSH into with `sudo`,
- **nginx** already running there, maybe for other sites,
- a **domain** you own, with a name for the hub pointed at the VPS. Below it's `chat.example.com`: add an `A` record
  (and `AAAA` if the VPS has IPv6) for it at your DNS provider ([using Cloudflare](#if-your-dns-is-on-cloudflare)).
  Replace `chat.example.com` everywhere with yours.

The plan: the hub listens only on `127.0.0.1` (the VPS itself), and nginx, which already owns ports 80 and 443,
passes `https://chat.example.com` through to it. The hub never faces the internet directly and can't clash with
your other apps.

```
 tablets / phones ──https──▶ nginx :443 (chat.example.com) ──▶ kitchat-hub 127.0.0.1:8787
                                    └── your other sites, unchanged
```

> These steps were checked by reading them through, not yet run end to end on a real VPS. If a step doesn't match
> what you see, trust what your server says and adjust.
>
> `KITCHAT_BIND`, `KITCHAT_TRUST_PROXY` and `KITCHAT_NO_PREVIEWS`, the rate limits (429) and the bad-token lockout
> need a hub build that has them; check with `/opt/kitchat/kitchat-hub --help`. An older hub ignores the
> environment settings it doesn't know, and `KITCHAT_BIND` then falls back to every network, so check that step 4's
> `ss` line shows `127.0.0.1`.

## 1. Pick a port

The hub needs one free port on the VPS, used only between nginx and the hub. See what's taken:

```bash
sudo ss -tlnp
```

If nothing is on `8787`, use it. Otherwise pick any free one between 1024 and 65535 (for example `8790`) and use it
wherever `8787` appears below.

Because the hub binds to `127.0.0.1`, that port is never reachable from outside, even with no firewall. The public
side is nginx on **443**, alongside your other sites; nginx tells them apart by name.

**A different public port** (optional): if you'd rather not share 443, nginx can also serve the hub on, say, `8443`.
The hub address in the app is then `chat.example.com:8443`. See the note at the end of step 6.

## 2. Build it on the VPS

Install Rust and the build tools (as your normal user, not root):

```bash
sudo apt update
sudo apt install -y build-essential pkg-config git sqlite3 rsync
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
```

Get the code and build only the hub:

```bash
git clone <your KitChat repo URL> ~/kitchat
cd ~/kitchat
cargo build -p kitchat-hub --release
```

A small VPS (1 GB of memory) can run out of memory while compiling. If the build is killed, add some swap and try
again:

```bash
sudo fallocate -l 2G /swapfile && sudo chmod 600 /swapfile && sudo mkswap /swapfile && sudo swapon /swapfile
```

Install it:

```bash
sudo install -d -m 755 /opt/kitchat
sudo install -m 755 target/release/kitchat-hub /opt/kitchat/kitchat-hub
```

## 3. A user and a folder for it

The hub runs as its own user, with no login and no home folder, and keeps everything in `/var/lib/kitchat`, which
only that user can read:

```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin kitchat
sudo install -d -m 700 -o kitchat -g kitchat /var/lib/kitchat
```

## 4. Run it with systemd

The settings go in one file:

```bash
sudo install -d -m 755 /etc/kitchat
sudo tee /etc/kitchat/hub.env > /dev/null <<'EOF'
KITCHAT_PORT=8787
KITCHAT_BIND=127.0.0.1
KITCHAT_DATA=/var/lib/kitchat
# nginx is in front: take the visitor's address from its X-Real-IP header (only from 127.0.0.1).
KITCHAT_TRUST_PROXY=1
# Uncomment to stop the hub fetching link previews:
# KITCHAT_NO_PREVIEWS=1
EOF
sudo chmod 644 /etc/kitchat/hub.env
```

`KITCHAT_TRUST_PROXY=1` matters: without it, every request looks like it comes from `127.0.0.1` (nginx), so the
hub's per-address limits would lump the whole world together.

The service, with the doors it doesn't need locked:

```bash
sudo tee /etc/systemd/system/kitchat-hub.service > /dev/null <<'EOF'
[Unit]
Description=KitChat family hub
After=network-online.target
Wants=network-online.target

[Service]
User=kitchat
Group=kitchat
EnvironmentFile=/etc/kitchat/hub.env
ExecStart=/opt/kitchat/kitchat-hub
Restart=on-failure
RestartSec=5
# The hub stops cleanly on SIGTERM (what `systemctl stop` sends).
TimeoutStopSec=20

# Hardening: the hub can write only to its data folder.
NoNewPrivileges=yes
ProtectSystem=strict
ReadWritePaths=/var/lib/kitchat
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
RestrictNamespaces=yes
RestrictRealtime=yes
LockPersonality=yes
# AF_UNIX/AF_NETLINK for looking up names (link previews); AF_INET/AF_INET6 for the network.
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK
SystemCallArchitectures=native
SystemCallFilter=@system-service
UMask=0077
# Big uploads stream to disk, so this is plenty for a family.
MemoryMax=512M

[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload
sudo systemctl enable --now kitchat-hub
```

Check it:

```bash
systemctl status kitchat-hub
sudo ss -tlnp | grep 8787          # should show 127.0.0.1:8787, not 0.0.0.0
curl -s http://127.0.0.1:8787/hello
```

`/hello` answers with something like `{"app":"kitchat-hub","protocol":…,"needs_setup":true,…}`. Its log is in
`journalctl -u kitchat-hub` (`-f` to follow it).

## 5. Set up the family, or bring your home hub over

Do **one** of these before step 6 makes the hub public.

### A new family

On first start the hub writes a setup code to `/var/lib/kitchat/SETUP-CODE.txt`:

```bash
sudo cat /var/lib/kitchat/SETUP-CODE.txt
```

While the hub isn't public yet, reach it from your PC through an SSH tunnel. On the PC (PowerShell has `ssh` built
in):

```bash
ssh -N -L 8787:127.0.0.1:8787 you@your-vps
```

Leave that running, then in the KitChat app on the same PC choose **Set up a new family** and use `localhost:8787` as
the hub address, plus the setup code. (If the home hub is still running on that PC, port 8787 is taken there: use
`-L 18787:127.0.0.1:8787` and `localhost:18787` instead.) Then stop the tunnel (Ctrl+C). After step 6, use **Family › This device ›
Change hub address** on that PC to switch it to `chat.example.com`, and give everyone else join codes with
`chat.example.com` as the address ([hub_setup.md](hub_setup.md) §3).

(You can also skip the tunnel and do the setup right after step 6. Then the hub is briefly public with an unused setup
code; the code is random and guesses are limited, so that's OK if you do it straight away.)

### Moving the hub from the home PC

Everything the hub knows is in its data folder (`hub-data`), so moving it is a copy:

1. On the home PC, **stop the hub** (end the task, or `Stop-ScheduledTask "KitChat hub"`, and make sure
   `kitchat-hub.exe` is gone from Task Manager). Don't start it again afterwards, or the two hubs drift apart.
2. On the VPS, stop the new hub and clear its empty data:

   ```bash
   sudo systemctl stop kitchat-hub
   sudo find /var/lib/kitchat -mindepth 1 -delete
   ```

3. Copy the folder across. From the home PC (PowerShell; `scp` is built in):

   ```bash
   scp -r C:\KitChat\hub-data you@your-vps:/tmp/
   ```

   (or `rsync -a hub-data/ you@your-vps:/tmp/hub-data/` from WSL or a Mac/Linux machine, which can resume.)
4. On the VPS, move it in place, fix the owner and start the hub:

   ```bash
   sudo rsync -a --remove-source-files /tmp/hub-data/ /var/lib/kitchat/
   sudo rm -rf /tmp/hub-data
   sudo chown -R kitchat:kitchat /var/lib/kitchat
   sudo chmod 700 /var/lib/kitchat
   sudo systemctl start kitchat-hub
   curl -s http://127.0.0.1:8787/hello      # "needs_setup":false and your family name
   ```

5. After step 6, on **each device** open **Family › This device › Change hub address** and enter
   `chat.example.com`. Nobody needs to re-join: the devices' sign-ins moved with the data.

## 6. nginx in front

### If your DNS is on Cloudflare

A DNS record can only point a name at an address, not at a port. So the record sends `chat.example.com` to the VPS,
and nginx (below) passes that name on to the hub's port. A subdomain of a domain you already use is fine; your other
sites aren't affected.

In Cloudflare, open your domain, then **DNS › Records**, and add:

| Type   | Name   | Content               | Proxy status             |
| ------ | ------ | --------------------- | ------------------------ |
| `A`    | `chat` | the VPS's IPv4 address | **DNS only** (grey cloud) |
| `AAAA` | `chat` | the VPS's IPv6 address (only if it has one) | **DNS only** (grey cloud) |

Check that it has arrived before running certbot. This should print the VPS's address:

```bash
getent hosts chat.example.com
```

This guide expects **DNS only**. With the orange cloud (Cloudflare's proxy) on, connections reach nginx from
Cloudflare's addresses, which changes a few things:

- Set Cloudflare's SSL/TLS mode to **Full (strict)**.
- Make nginx restore the visitor's address, or the limits in this guide treat everyone as a few shared addresses: add
  `set_real_ip_from` for each of [Cloudflare's address ranges](https://www.cloudflare.com/ips/), plus
  `real_ip_header CF-Connecting-IP;`.
- The fail2ban jail in step 8 won't block anyone, because the firewall sees Cloudflare rather than the visitor.

### Limits, site and certificate

First, request limits that nginx applies before anything reaches the hub. These go in the `http` level, so they get
their own file:

```bash
sudo tee /etc/nginx/conf.d/kitchat-limits.conf > /dev/null <<'EOF'
# KitChat: per-address limits (a first layer; the hub has its own too).
limit_req_zone  $binary_remote_addr zone=kitchat:10m      rate=20r/s;
limit_req_zone  $binary_remote_addr zone=kitchat_join:10m rate=10r/m;
limit_conn_zone $binary_remote_addr zone=kitchat_conn:10m;

map $http_upgrade $kitchat_connection_upgrade {
    default upgrade;
    ''      close;
}
EOF
```

A family at home shares one internet address, so these are generous: opening a chat full of photos makes many
requests at once.

Then the site, with only the plain-http part for now, so certbot can prove you own the name:

```bash
sudo tee /etc/nginx/sites-available/kitchat > /dev/null <<'EOF'
server {
    listen 80;
    listen [::]:80;
    server_name chat.example.com;

    location / {
        return 301 https://$host$request_uri;
    }
}
EOF
sudo ln -s /etc/nginx/sites-available/kitchat /etc/nginx/sites-enabled/kitchat
sudo nginx -t && sudo systemctl reload nginx
```

Get a certificate (if certbot isn't installed yet: `sudo apt install -y certbot python3-certbot-nginx`):

```bash
sudo certbot certonly --nginx -d chat.example.com --deploy-hook "systemctl reload nginx"
```

certbot renews it on its own; the deploy hook reloads nginx so the new certificate is used. `sudo certbot renew
--dry-run` checks that renewal works.

Now add the https part to the same file:

```bash
sudo tee -a /etc/nginx/sites-available/kitchat > /dev/null <<'EOF'

server {
    listen 443 ssl;
    listen [::]:443 ssl;
    server_name chat.example.com;

    ssl_certificate     /etc/letsencrypt/live/chat.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/chat.example.com/privkey.pem;
    ssl_protocols       TLSv1.2 TLSv1.3;

    add_header Strict-Transport-Security "max-age=31536000" always;

    access_log /var/log/nginx/kitchat.access.log;
    error_log  /var/log/nginx/kitchat.error.log;

    # Big files go up in 4 MB chunks; small photos in one request of up to 15 MB.
    client_max_body_size 20m;

    limit_req  zone=kitchat burst=200 nodelay;
    limit_conn kitchat_conn 50;
    limit_req_status  429;
    limit_conn_status 429;

    proxy_http_version 1.1;
    proxy_set_header Host            $host;
    proxy_set_header X-Real-IP       $remote_addr;
    proxy_set_header X-Forwarded-For $remote_addr;
    proxy_set_header X-Forwarded-Proto https;

    location / {
        proxy_pass http://127.0.0.1:8787;
        proxy_request_buffering off;   # stream uploads straight to the hub
        proxy_read_timeout 120s;
        proxy_send_timeout 120s;
    }

    # New messages arrive over a WebSocket that stays open.
    location = /live {
        proxy_pass http://127.0.0.1:8787;
        proxy_set_header Upgrade    $http_upgrade;
        proxy_set_header Connection $kitchat_connection_upgrade;
        proxy_read_timeout 1h;
        proxy_send_timeout 1h;
        proxy_buffering off;
    }

    # Joining and first setup: few tries per address.
    location ~ ^/(enroll|setup)$ {
        limit_req zone=kitchat_join burst=5 nodelay;
        proxy_pass http://127.0.0.1:8787;
    }
}
EOF
sudo nginx -t && sudo systemctl reload nginx
```

Try it from anywhere: `https://chat.example.com/hello` in a browser shows the `{"app":"kitchat-hub",...}` line. In
the app the hub address is just `chat.example.com`; the app uses https for any address that isn't on the home
network.

**HSTS** tells browsers to only ever use https for this name. It's set for this name only (no `includeSubDomains`), so
it doesn't affect your other sites.

**On a different public port** (optional): change the two `listen 443` lines to `listen 8443 ssl;` and
`listen [::]:8443 ssl;`, allow that port in step 7 (`sudo ufw allow 8443/tcp`) and use `chat.example.com:8443` in the
app. Keep the port-80 block: certbot uses it to renew the certificate.

## 7. Firewall

Only SSH, http and https need to be open. If ufw isn't on yet, **allow SSH first** so you don't lock yourself out:

```bash
sudo ufw allow OpenSSH
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
sudo ufw enable
sudo ufw status verbose
```

If ufw was already on for your other sites, just check the list: nothing should be open that you don't recognise.
The hub's port must **not** be in it (it's on `127.0.0.1` anyway).

Watch out for **Docker**: ports published by Docker containers (`-p 8080:8080`) skip ufw entirely. If you run other
apps in Docker, publish them as `-p 127.0.0.1:8080:8080` and put nginx in front, like the hub.

## 8. fail2ban for code guessing

The hub already locks out repeated wrong join codes and bad tokens. fail2ban adds a firewall ban for an address
that keeps trying, so it stops reaching nginx at all.

```bash
sudo apt install -y fail2ban

sudo tee /etc/fail2ban/filter.d/kitchat.conf > /dev/null <<'EOF'
[Definition]
# Wrong setup or join codes (400/401) and lockouts (429) on /setup and /enroll.
failregex = ^<HOST> \S+ \S+ \[[^\]]+\] "POST /(?:setup|enroll)(?:\?\S*)? HTTP/[\d.]+" (?:400|401|429) 
ignoreregex =
EOF

sudo tee /etc/fail2ban/jail.d/kitchat.conf > /dev/null <<'EOF'
[kitchat]
enabled  = true
port     = http,https
filter   = kitchat
logpath  = /var/log/nginx/kitchat.access.log
backend  = auto
maxretry = 10
findtime = 10m
bantime  = 1h
EOF

sudo systemctl restart fail2ban
sudo fail2ban-client status kitchat
```

Test the filter against the log: `sudo fail2ban-regex /var/log/nginx/kitchat.access.log kitchat`. A family member who
types a code wrong a few times won't be banned; ten wrong tries in ten minutes will be. To let someone back in:
`sudo fail2ban-client set kitchat unbanip 203.0.113.7`.

(If you moved the hub to port 8443, put `port = http,https,8443`.)

## 9. Server basics

Worth doing once if they aren't done already:

- **SSH keys only.** With your key working, in `/etc/ssh/sshd_config` (or a file in `/etc/ssh/sshd_config.d/`) set
  `PasswordAuthentication no` and `PermitRootLogin no`, then `sudo systemctl reload ssh`. Keep your current session
  open and test a new login before closing it.
- **Security updates on their own:**

  ```bash
  sudo apt install -y unattended-upgrades
  sudo dpkg-reconfigure -plow unattended-upgrades
  ```

- **Keep logs from filling the disk:**

  ```bash
  sudo mkdir -p /etc/systemd/journald.conf.d
  printf '[Journal]\nSystemMaxUse=500M\n' | sudo tee /etc/systemd/journald.conf.d/size.conf
  sudo systemctl restart systemd-journald
  ```

- **Watch disk space.** Full-size photos and videos add up (a minute of phone video is 100-200 MB). Check with
  `df -h /var/lib/kitchat` and `sudo du -sh /var/lib/kitchat/media`. A tiny daily warning by email or log, if you
  like: `df --output=pcent /var/lib/kitchat | tail -1` above 85% is the time to clean up or grow the disk. Many VPS
  providers can also send an alert when the disk is nearly full.

## 10. Backups

The hub keeps everything in `/var/lib/kitchat`: the database `kitchat-hub.db` and the `media` folder with every photo
and video. A nightly copy, made safely while the hub keeps running:

```bash
sudo install -d -m 700 -o kitchat -g kitchat /var/backups/kitchat

sudo tee /opt/kitchat/backup.sh > /dev/null <<'EOF'
#!/bin/sh
set -eu
SRC=/var/lib/kitchat
DEST=/var/backups/kitchat
# A consistent copy of the database even while the hub is writing to it.
sqlite3 "$SRC/kitchat-hub.db" ".backup '$DEST/kitchat-hub.db.tmp'"
mv "$DEST/kitchat-hub.db.tmp" "$DEST/kitchat-hub.db"
# Media files never change once written, so rsync only copies new ones.
rsync -a --delete "$SRC/media/" "$DEST/media/"
EOF
sudo chmod 755 /opt/kitchat/backup.sh

sudo tee /etc/systemd/system/kitchat-backup.service > /dev/null <<'EOF'
[Unit]
Description=Back up the KitChat hub

[Service]
Type=oneshot
User=kitchat
Group=kitchat
ExecStart=/opt/kitchat/backup.sh
EOF

sudo tee /etc/systemd/system/kitchat-backup.timer > /dev/null <<'EOF'
[Unit]
Description=Back up the KitChat hub every night

[Timer]
OnCalendar=*-*-* 03:30
Persistent=true
RandomizedDelaySec=15m

[Install]
WantedBy=timers.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable --now kitchat-backup.timer
sudo systemctl start kitchat-backup.service    # one now, to check it works
ls -la /var/backups/kitchat
```

A backup on the same server doesn't help if the server is lost, so also copy `/var/backups/kitchat` somewhere else
now and then, for example to the home PC:

```bash
scp -r you@your-vps:/var/backups/kitchat C:\KitChat\vps-backup
```

(That needs your SSH user to be able to read the folder, e.g. `sudo setfacl -R -m u:you:rX /var/backups/kitchat`
after `sudo apt install acl`, or copy with rsync from a machine where you can `sudo`.) Your VPS provider's snapshots
are a good extra layer.

**Restore:**

```bash
sudo systemctl stop kitchat-hub
sudo rm -f /var/lib/kitchat/kitchat-hub.db-wal /var/lib/kitchat/kitchat-hub.db-shm
sudo cp /var/backups/kitchat/kitchat-hub.db /var/lib/kitchat/kitchat-hub.db
sudo rsync -a /var/backups/kitchat/media/ /var/lib/kitchat/media/
sudo chown -R kitchat:kitchat /var/lib/kitchat
sudo systemctl start kitchat-hub
```

Messages sent after the backup are gone from the hub, but devices still show what they had cached.

## 11. Updating, checking, trouble

**Updating the hub:**

```bash
cd ~/kitchat && git pull && cargo build -p kitchat-hub --release
sudo install -m 755 target/release/kitchat-hub /opt/kitchat/kitchat-hub
sudo systemctl restart kitchat-hub
curl -s https://chat.example.com/hello
```

The devices reconnect on their own within a few seconds. Running the backup first (`sudo systemctl start
kitchat-backup.service`) is a good habit before a big update.

**Is it all working?** `https://chat.example.com/hello` from a phone on mobile data should show
`{"app":"kitchat-hub",...}`.

**Trouble:**

| What you see                                   | Look at                                                                         |
| ---------------------------------------------- | ------------------------------------------------------------------------------- |
| `502 Bad Gateway`                              | The hub isn't running or is on another port: `systemctl status kitchat-hub`, `journalctl -u kitchat-hub -n 50`, and the port in `hub.env` vs `proxy_pass`. |
| Hub won't start, "Couldn't listen"             | The port is taken: `sudo ss -tlnp`, pick another (step 1).                     |
| Hub won't start, "Couldn't open the hub's data"| Ownership: `sudo chown -R kitchat:kitchat /var/lib/kitchat`.                   |
| Certificate error in the app or browser        | `sudo certbot certificates`; is DNS for `chat.example.com` pointing at this VPS? |
| Messages only arrive when the app is reopened  | The `/live` WebSocket: check the `location = /live` block and `kitchat.error.log`. |
| Big videos fail, small photos fine             | `client_max_body_size` too small (look for `413` in `kitchat.access.log`).     |
| `429` errors for the whole family              | nginx limits too tight for one shared home address: raise `burst` in step 6.   |
| Someone got banned                             | `sudo fail2ban-client status kitchat`, then `unbanip` (step 8).                |
| Another site broke after adding this one       | `sudo nginx -t`; each site needs its own `server_name`.                        |

For more detail from the hub, add `RUST_LOG=debug` to `/etc/kitchat/hub.env` and `sudo systemctl restart
kitchat-hub`; remove it again afterwards.

## 12. Other ways

- **Tailscale only**: run the hub on the VPS as above but without nginx, with `KITCHAT_BIND` set to the VPS's
  Tailscale address (`100.x.y.z`), and install Tailscale on each family device. Nothing is public at all; every device
  needs the Tailscale app running. See [hub_setup.md](hub_setup.md) §5.
- **Cloudflare Tunnel**: `cloudflared` on the VPS connects out to Cloudflare, which serves `https://chat.example.com`
  for you, so ports 80 and 443 can stay closed and certbot isn't needed. Point the tunnel at
  `http://localhost:8787` (setup in [hub_setup.md](hub_setup.md) §5). With a tunnel the hub sees every request coming
  from `127.0.0.1`. Leave `KITCHAT_TRUST_PROXY` **off** there: Cloudflare passes an `X-Real-IP` header from the
  visitor through unchanged, so anyone could pretend to be any address. The hub's limits then count everyone
  together, so keep the tunnel for a small family or put nginx between the tunnel and the hub.
