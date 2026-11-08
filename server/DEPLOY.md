# Deploying the Reqora sync server

This guide sets up the sync server on a Linux machine with a public address, behind HTTPS,
so Reqora on every device can sign in to it. It takes about 15 minutes.

The setup in [`docker-compose.yml`](docker-compose.yml) runs three containers:

```
Reqora app ──HTTPS──▶ Caddy :443 ──▶ server :8000 ──▶ PostgreSQL
                      (certificate      (FastAPI)       (data volume)
                       from Let's Encrypt)
```

Only Caddy is reachable from outside. The database and the server itself are not exposed.

Other setups (an existing nginx, a home network without a domain, no Docker) are covered
[at the end](#other-setups).

## What you need

- A Linux server with a public IP address. 1 vCPU, 1 GB RAM and 10 GB of disk are plenty for a team.
  The commands below are for Ubuntu 24.04 / Debian 12.
- A domain or subdomain for it, e.g. `sync.example.com`.
- SSH access with `sudo`.
- Ports 80 and 443 open to the internet. If your provider has a cloud firewall or security groups,
  allow them there too. Caddy needs port 80 to obtain the certificate.

## 1. Point the domain at the server

At your DNS provider, create an `A` record for `sync.example.com` with the server's IPv4 address
(and an `AAAA` record for IPv6 if the server has one). Check from your computer:

```bash
dig +short sync.example.com      # must print the server's IP
```

DNS changes can take a few minutes to show up. Don't continue until this prints the right address,
or the certificate request in step 4 will fail.

## 2. Prepare the server

```bash
sudo apt update && sudo apt upgrade -y

# Docker Engine and the Compose plugin
curl -fsSL https://get.docker.com | sudo sh
sudo usermod -aG docker "$USER"
# Log out and back in so the group change applies, then check:
docker compose version
```

Firewall (skip it if your provider's firewall already does this):

```bash
sudo ufw allow OpenSSH
sudo ufw allow 80/tcp
sudo ufw allow 443        # TCP for HTTPS, UDP for HTTP/3
sudo ufw enable
```

Ports published by Docker bypass `ufw`. That's fine here, since the compose file only publishes 80 and 443.
Don't add a `ports:` entry to the `db` service.

## 3. Get the code and configure it

```bash
git clone https://github.com/mirosgit/reqora.git
cd reqora/server
cp .env.example .env
sed -i "s/^POSTGRES_PASSWORD=.*/POSTGRES_PASSWORD=$(openssl rand -hex 24)/" .env
chmod 600 .env
nano .env                 # set REQORA_DOMAIN=sync.example.com
```

`.env` ends up looking like this:

```ini
REQORA_DOMAIN=sync.example.com
POSTGRES_PASSWORD=3f9c…            # generated above
```

Signing up is **invite-only**: nobody can create an account on your server unless you give them a code
(step 5). Leave it that way for a server on the internet.

## 4. Start it

```bash
docker compose up -d
docker compose ps                 # db, server and caddy should be "running" / "healthy"
docker compose logs -f caddy      # wait for "certificate obtained successfully", then Ctrl+C
```

The first start builds the server image and creates the database tables (about a minute).
Check from your computer:

```bash
curl https://sync.example.com/health
# {"status":"ok","version":"0.1.0"}
```

## 5. Create your account

Accounts are created with invite codes, which only someone with access to the server can make.
Make one for yourself:

```bash
docker compose exec server python -m reqora_server.admin create-invite --email you@example.com
# Invite code for you@example.com, valid until 2026-11-15 20:53 UTC:
#   SC3N-3XSH-MS7E
```

Then in Reqora: **⚙ → Sync & account…** (or click **Sync off** in the status bar).

1. Server: `https://sync.example.com`, then **Check**. It should say "Reqora sync server 0.1.0 · sign-up by invite".
2. Choose **Create account**, enter your email, a password and the **invite code**.
3. Leave **Upload this device's data** on to merge what's already on the device into the account.

On your other devices, choose **Sign in** with the same email and password; no code is needed.

From then on, changes sync a couple of seconds after they're saved, and every 30 seconds the app checks
for changes made on other devices. **Sync now** in the same dialog forces a sync.

## 6. Invite other people

For each person, create a code and send it to them (by a channel you trust, it's as good as a key to
the server):

```bash
docker compose exec server python -m reqora_server.admin create-invite --email bob@example.com --note "Bob, QA"
```

- A code works **once**, and expires after 7 days (`--days N` to change).
- With `--email`, only that address can use it. Without it, whoever has the code can sign up with any
  email, so prefer `--email`.
- `--count 5` makes several codes at once (without `--email`).
- `list-invites` shows the open codes (`--all` also used and expired ones, and who used them);
  `revoke-invite ID` cancels one that hasn't been used.

Only a hash of each code is stored, so a code can't be looked up later: if one is lost, revoke it and make
a new one.

## Backups

All data lives in PostgreSQL. A nightly dump with two weeks of history:

```bash
sudo mkdir -p /var/backups/reqora && sudo chown "$USER" /var/backups/reqora
crontab -e
```

Add this line, with the path to your `reqora/server` directory:

```cron
15 3 * * * cd /home/USER/reqora/server && docker compose exec -T db pg_dump -U reqora --clean --if-exists reqora | gzip > /var/backups/reqora/reqora-$(date +\%F).sql.gz && find /var/backups/reqora -name '*.sql.gz' -mtime +14 -delete
```

Copy the dumps somewhere off the server too (another machine, object storage). They contain everything
users synced, **including tokens and passwords saved in requests and environments**, so store them
like secrets.

Restoring a dump:

```bash
docker compose stop server
gunzip -c /var/backups/reqora/reqora-2026-11-08.sql.gz \
  | docker compose exec -T db psql -q -U reqora -d reqora -v ON_ERROR_STOP=1
docker compose start server
```

After a restore, the apps notice that the server went back in time and download everything again.
Changes they made since the backup are merged back in on their next sync.

## Updating

Back up first, then:

```bash
cd reqora && git pull
cd server && docker compose up -d --build
```

Database migrations run automatically when the server container starts.

## Managing accounts

The server comes with a small admin tool:

```bash
docker compose exec server python -m reqora_server.admin create-invite --email ada@example.com
docker compose exec server python -m reqora_server.admin list-invites
docker compose exec server python -m reqora_server.admin list-users
docker compose exec server python -m reqora_server.admin create-user ada@example.com --name "Ada Lovelace"
docker compose exec server python -m reqora_server.admin set-password ada@example.com
docker compose exec server python -m reqora_server.admin delete-user ada@example.com
```

`create-user` makes an account directly with a password you choose (the person can change it in the app
afterwards); an invite is usually better because they pick the password themselves. `create-user` and
`set-password` ask for the password. `set-password` also signs the account out on all of its devices,
which is what to do when someone forgets their password.

Users manage their own devices and password in the app: **⚙ → Sync & account… → Devices** lists every
device signed in to the account and signs any of them out (e.g. a lost laptop), and **Password** changes the
password, which also signs out all other devices. The same works through the API:

```bash
API=https://sync.example.com/api/v1
TOKEN=$(curl -s $API/auth/login -H 'Content-Type: application/json' \
  -d '{"email": "ada@example.com", "password": "…"}' | python3 -c 'import sys, json; print(json.load(sys.stdin)["token"])')
curl -s $API/auth/sessions -H "Authorization: Bearer $TOKEN"                        # list devices
curl -s -X DELETE $API/auth/sessions/SESSION_ID -H "Authorization: Bearer $TOKEN"   # sign one out
```

The full API is in [README.md](README.md#api) and at `https://sync.example.com/docs`.

## Day-to-day

| Task | Command |
|---|---|
| Status | `docker compose ps` |
| Server logs | `docker compose logs -f server` |
| Certificate / proxy logs | `docker compose logs -f caddy` |
| Restart everything | `docker compose restart` |
| Stop / start | `docker compose down` / `docker compose up -d` (data is kept in volumes) |
| Disk used by the database | `docker compose exec db psql -U reqora -c "select pg_size_pretty(pg_database_size('reqora'))"` |

Caddy renews the certificate by itself. Keep the OS patched; `sudo apt install unattended-upgrades`
does it automatically.

## Other setups

### You already run nginx (or another reverse proxy)

Remove the `caddy` service (and the `caddy_data` / `caddy_config` volumes) from `docker-compose.yml`,
and publish the server on localhost instead:

```yaml
  server:
    # ...
    ports:
      - "127.0.0.1:8000:8000"
```

Then proxy to it. For nginx (certificate from certbot or similar):

```nginx
server {
    listen 443 ssl http2;
    server_name sync.example.com;
    ssl_certificate     /etc/letsencrypt/live/sync.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/sync.example.com/privkey.pem;

    # nginx allows 1 MB bodies by default; a sync push can be larger.
    client_max_body_size 32m;

    location / {
        proxy_pass http://127.0.0.1:8000;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

`REQORA_DOMAIN` is not needed in `.env` then. Remove that line or leave it set to anything.

### Home server or LAN, no domain

For a few devices on one network, a single container with SQLite is enough:

```bash
cd reqora/server
docker build -t reqora-server .
docker run -d --name reqora-sync --restart unless-stopped \
  -p 8000:8000 -v reqora-data:/data reqora-server
```

In Reqora, use `http://192.168.1.10:8000` (the machine's LAN address). The app warns that the connection
isn't encrypted. That's acceptable inside a home network, but never expose a plain-http server to
the internet. Back up the `reqora-data` volume (it holds `reqora.db`), and run the admin tool with
`docker exec -it reqora-sync python -m reqora_server.admin ...` (e.g. `create-invite`).
On a network where you trust everyone, `-e REQORA_OPEN_REGISTRATION=true` lets anyone who can reach the
server sign up without a code.

### Without Docker

Requires Python 3.11+ (`sudo apt install python3-venv git`).

```bash
sudo useradd --system --create-home --home-dir /opt/reqora --shell /usr/sbin/nologin reqora
sudo -u reqora git clone https://github.com/mirosgit/reqora.git /opt/reqora/src
sudo -u reqora python3 -m venv /opt/reqora/venv
sudo -u reqora /opt/reqora/venv/bin/pip install /opt/reqora/src/server
echo 'REQORA_DATABASE_URL=sqlite+aiosqlite:////opt/reqora/reqora.db' | sudo tee /etc/reqora.env
```

PostgreSQL works too: install it, create a database and a user, and set
`REQORA_DATABASE_URL=postgresql+asyncpg://user:password@localhost/reqora`.

`/etc/systemd/system/reqora.service`:

```ini
[Unit]
Description=Reqora sync server
After=network.target

[Service]
User=reqora
WorkingDirectory=/opt/reqora/src/server
EnvironmentFile=/etc/reqora.env
ExecStartPre=/opt/reqora/venv/bin/alembic upgrade head
ExecStart=/opt/reqora/venv/bin/uvicorn reqora_server.main:create_app --factory --host 127.0.0.1 --port 8000 --proxy-headers
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload && sudo systemctl enable --now reqora
```

Put HTTPS in front of it, e.g. Caddy from the distribution (`sudo apt install caddy`) with
`/etc/caddy/Caddyfile`:

```
sync.example.com {
    reverse_proxy 127.0.0.1:8000
}
```

and `sudo systemctl reload caddy`. To update: `git pull` in `/opt/reqora/src`, reinstall with `pip`,
then `sudo systemctl restart reqora`.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `curl https://…/health` fails, Caddy logs mention ACME or a challenge | DNS doesn't point to this server yet, or ports 80/443 are blocked (check the provider's firewall). Fix it, then `docker compose restart caddy`. |
| App: "No Reqora sync server at this address" | Wrong address, or a proxy in front doesn't forward `/api/v1/…`. Use the base URL, e.g. `https://sync.example.com`. |
| App: "Cannot connect to the server: … certificate …" | The server has a self-signed or expired certificate. The app only accepts publicly trusted certificates (Caddy gets one automatically). |
| App: "Cannot connect to the server" | The server is down (`docker compose ps`), or a firewall blocks the port. |
| App: "This server is invite-only" | Create an invite (`create-invite`) and enter the code under **Create account**. |
| App: "This invite code is invalid, expired or already used" | Codes work once and expire (7 days by default), and a code made with `--email` only works for that email. Check with `list-invites --all`, then make a new one. |
| App: "Too many attempts" | More than 10 sign-in attempts per minute from one IP. Wait a minute. If everyone hits it at once, the proxy isn't passing the client IP (see the next row). |
| Server logs show the proxy's address for every request | The server doesn't trust the proxy's `X-Forwarded-For`. Set `FORWARDED_ALLOW_IPS` on the server to the proxy's address (the compose file already does this for Caddy). |
| nginx returns `413 Request Entity Too Large` | Add `client_max_body_size 32m;` (see above). |
| App: "N items are too large for the server" | A request (usually a big raw body) is over the 1 MB document limit. Raise `REQORA_MAX_DOCUMENT_BYTES` in the server's environment, or keep such bodies in files (binary / form-data file fields). |

## Security checklist

- [ ] Reachable only over HTTPS (Caddy, or your own proxy with a real certificate)
- [ ] `POSTGRES_PASSWORD` generated randomly, `.env` readable only by you (`chmod 600 .env`)
- [ ] Database port not published
- [ ] Sign-up stays invite-only (`REQORA_OPEN_REGISTRATION` not set), invites made with `--email`
- [ ] Nightly backups, also kept off the server, stored like secrets
- [ ] OS security updates installed automatically
