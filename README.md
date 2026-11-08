# Reqora

A fast, lightweight open-source API client (a Postman alternative) for Linux, Windows and macOS,
written in Rust: the GUI is built with [egui](https://github.com/emilk/egui) (GPU-rendered), HTTP runs on
`reqwest` + `tokio`. It ships as a single binary and needs no system `-dev` packages. The Inter font and
Phosphor icons are bundled, so the interface looks the same on every machine.

**Want the same collections on several computers?** Follow the step-by-step guide in
[Sync between computers](#sync-between-computers).

## Features

- **Requests**: GET / POST / PUT / PATCH / DELETE / HEAD / OPTIONS, multiple tabs
- **Params**: kept in sync with the URL bar both ways; individual params can be disabled
- **Headers**: key–value table with checkboxes
- **Body**: `none`, `form-data` (text and files), `x-www-form-urlencoded`, `raw` (JSON / Text / XML / HTML / JS, with highlighting and a Beautify button), `binary` (file)
- **Authorization**: Bearer Token, Basic Auth, API Key (in a header or in the query)
- **Settings**: timeout, follow redirects, disable SSL verification (for self-signed certificates)
- **Response**: status, time, size. JSON and XML are formatted and highlighted,
  images are shown as a preview. Raw mode, search (Ctrl+F), copy, save to file,
  response headers and the headers of the sent request. The viewer only draws visible lines,
  so multi-megabyte responses stay smooth
- **Comments** on requests and collections: the 💬 button in the request header opens a discussion panel
  with author, timestamp, editing and deletion. Post with `Ctrl+Enter`. Set the author name in the ⚙ menu.
  Comments are saved immediately and don't mark the request as unsaved
- **Collections**: save (Ctrl+S), rename, duplicate, delete, filter.
  Double-clicking a collection (or "Overview & comments" in its menu) opens an overview:
  description, collection variables, list of requests, and comments
- **Variables**:
  - `{{base_url}}` works in the URL, params, headers, body, and authorization;
  - `{base_url}` (single braces) works in the URL if such a variable is defined.
    Unknown `{id}` placeholders are left as is, so templates like `/users/{id}` don't break;
  - variables come from the request's collection and from the active environment; the environment wins;
  - in the URL bar, known variables are highlighted green and unknown ones red;
  - built-ins: `{{$guid}}`, `{{$timestamp}}`, `{{$isoTimestamp}}`, `{{$randomInt}}`
- **History** of every sent request, grouped by day
- **Sync between devices** through a self-hosted sync server: ⚙ → **Sync & account…**, enter the
  server's address, sign in or create an account. Collections, requests, environments, history and your
  comment name stay in sync; edits made on two devices are merged field by field. See [Sync between computers](#sync-between-computers)
- **cURL**: paste `curl ...` into the URL bar and the request is imported. The other way: the "cURL" button
- **Postman import/export**: v2.0/v2.1 collections (including variables and description) and environments
- Cookies persist between requests within a session (reset from the status bar or the ⚙ menu)
- Dark, light, or system theme; the response can be placed below or to the right
- Open tabs, including unsaved edits, are restored after a restart

## Keyboard shortcuts

On macOS, use ⌘ instead of Ctrl (except `Ctrl+Tab`, which stays Ctrl). The app shows the right keys
for your platform.

| Keys | Action |
|---|---|
| `Ctrl+Enter` | Send request (in a comment field: post the comment) |
| `Ctrl+S` / `Ctrl+Shift+S` | Save / Save as |
| `Ctrl+T` / `Ctrl+W` | New / close tab |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |
| `Ctrl+L` | Focus the URL bar |
| `Ctrl+F` | Search in response |
| `Ctrl+B` | Show / hide sidebar |

## Installation

Building from source requires Rust (if you don't have it: <https://rustup.rs>).
The build is limited to half of the CPU cores and runs at low priority so it doesn't freeze the system.
You can set the number of jobs manually with the `JOBS` environment variable, e.g. `JOBS=2 ./install.sh`.

### Linux

```bash
./install.sh          # builds a release and installs it into ~/.local (reqora binary, menu entry, icon)
./uninstall.sh        # uninstall (data is kept; --purge removes it too)
```

Build a `.deb`:

```bash
cargo build --release
./packaging/build-deb.sh          # → dist/reqora_<version>_amd64.deb
sudo apt install ./dist/reqora_*.deb
```

### macOS

Requires the Xcode Command Line Tools (`xcode-select --install`).

```bash
./install.sh          # builds Reqora.app and installs it into ~/Applications (APPS_DIR=/Applications to change)
./uninstall.sh        # uninstall (data is kept; --purge removes it too)
```

Build a `.dmg`:

```bash
cargo build --release
./packaging/build-macos.sh        # → dist/Reqora.app and dist/reqora_<version>_macos_<arch>.dmg
```

The app is ad-hoc signed, not notarized. If macOS refuses to open a downloaded copy, allow it in
System Settings → Privacy & Security → "Open Anyway", or run `xattr -dr com.apple.quarantine /Applications/Reqora.app`.

### Windows

Requires the Rust toolchain for MSVC (rustup installs it together with the Visual Studio Build Tools).
In PowerShell:

```powershell
.\install.ps1         # builds and installs into %LOCALAPPDATA%\Programs\Reqora, adds a Start menu shortcut
.\uninstall.ps1       # uninstall (data is kept; -Purge removes it too)
```

If running scripts is disabled: `powershell -ExecutionPolicy Bypass -File .\install.ps1`.

Build a portable `.zip`:

```powershell
cargo build --release
.\packaging\build-windows.ps1    # → dist\reqora_<version>_windows_x64.zip
```

The executable is not code-signed, so SmartScreen may warn on first launch: "More info" → "Run anyway".

### Without installing

`cargo run --release` works on every platform.

### Prebuilt packages

CI (`.github/workflows/ci.yml`) runs the tests on Linux, Windows and macOS for every push and pull request,
and attaches the `.deb`, the Windows `.zip` and a universal (Apple Silicon + Intel) macOS `.dmg` to the run
as artifacts.

## Sync between computers

A step-by-step guide. No experience with servers needed: copy the commands, check that you see what the
guide says you should see, and move on.

### How it works

- A **sync server** is a small program that keeps a copy of your collections, environments and history.
  Reqora on each of your computers sends its changes there and picks up the changes made on the others.
- **You run the server yourself**, on your own computer or on a rented one. Your data doesn't go anywhere else.
- **Strangers can't get in.** An account can only be created with an **invite code**, and only the person
  who has access to the server can make one.
- Each person has their **own account and their own data**. Accounts don't see each other's collections.

Pick one:

- **[Option A](#option-a-try-it-on-this-computer)**: the server runs on this computer. Takes 10 minutes and
  is great for trying it out, but only this computer (or your home network) can use it.
- **[Option B](#option-b-a-real-server-on-the-internet)**: the server runs on a rented machine on the internet,
  so your laptop, office PC and so on sync from anywhere. Takes about 30 minutes.

### Option A: try it on this computer

You need **Python 3.11 or newer**. Check in a terminal:

```bash
python3 --version          # must say 3.11 or higher
```

If it's older or missing, install Python from <https://www.python.org/downloads/>.

**Step 1. Get the code.** Skip this if you already have the `reqora` folder (for example because you
installed Reqora from it).

```bash
git clone https://github.com/mirosgit/reqora.git
```

**Step 2. Install the server.** Only needed once. Run this from the folder that contains `reqora`
(if you skipped step 1, go to wherever your `reqora` folder is).

```bash
cd reqora/server
python3 -m venv .venv
.venv/bin/pip install .
.venv/bin/alembic upgrade head
```

At the end you should see two lines like `Running upgrade -> 0001` and `Running upgrade 0001 -> 0002`.
This creates the file `reqora.db` in the `server` folder: that's where all synced data is kept.

> On **Windows**, write `.venv\Scripts\` instead of `.venv/bin/` in every command,
> e.g. `.venv\Scripts\pip install .`, and `python` instead of `python3`.

**Step 3. Start the server.**

```bash
.venv/bin/uvicorn reqora_server.main:create_app --factory
```

You should see `Uvicorn running on http://127.0.0.1:8000`. **Leave this terminal window open**: the server
runs as long as it's open. To stop it, press `Ctrl+C` in it. To start it again later, open a terminal in
the `server` folder and run this step again (steps 2 and 4 are not needed again).

If you see `address already in use` instead, another program is using port 8000. Start it on another port,
and use that port in the address in step 5:

```bash
.venv/bin/uvicorn reqora_server.main:create_app --factory --port 8001
```

**Step 4. Make an invite code for yourself.** Open a **second** terminal, go to the same `server` folder,
and run (with your own email):

```bash
.venv/bin/python -m reqora_server.admin create-invite --email you@example.com
```

It prints the code:

```
Invite code for you@example.com, valid until 2026-11-15 21:03 UTC:
  DSZQ-QXX8-5NCG
```

Keep it for the next step. It works once and expires in 7 days; if anything goes wrong, just make another one.

**Step 5. Create your account in Reqora.**

1. Open Reqora and click **Sync off** at the bottom left (or ⚙ at the top right → **Sync & account…**).
2. In **Server**, type `http://127.0.0.1:8000` and click **Check**.
   A green line appears: **Reqora sync server 0.1.0 · sign-up by invite**.
3. Click the **Create account** tab and fill in:
   - **Email**: the same one as in step 4
   - **Password**: at least 8 characters (you'll need it on your other computers)
   - **Your name**: shown on your comments
   - **Invite code**: the code from step 4
4. Leave **Upload this device's data** switched on: the collections already on this computer go into
   your account.
5. Click **Create account**.

The bottom left now says **Synced** in green. Done: from now on, everything you save is synced a couple
of seconds later.

**Other computers at home?** Stop the server (`Ctrl+C`) and start it so the rest of your network can
reach it:

```bash
.venv/bin/uvicorn reqora_server.main:create_app --factory --host 0.0.0.0
```

On the other computers, use `http://<this computer's IP>:8000` as the server (the IP is shown by
`hostname -I` on Linux, `ipconfig getifaddr en0` on macOS, `ipconfig` on Windows). Reqora warns that
the connection isn't encrypted: that's fine inside your home network, but never expose a server like this
to the internet; use Option B for that. If they can't connect, the firewall on this computer is blocking
port 8000 (on Ubuntu: `sudo ufw allow 8000/tcp`).

### Option B: a real server on the internet

You need:

- **A rented Linux server (VPS)** from any hosting provider, running Ubuntu 24.04 or Debian 12.
  The cheapest plan (1 CPU, 1 GB RAM) is plenty. You get its **IP address** and SSH access.
- **A domain name** you control, for example `sync.example.com`. At your domain provider, add an
  **A record** for it pointing to the server's IP address. After a few minutes, `ping sync.example.com`
  should show that IP.
- **Ports 80 and 443 open.** If your provider has a firewall in its control panel, allow them there.

Connect to the server (`ssh user@server-ip`) and run the following. **Replace `sync.example.com` with
your domain** everywhere.

```bash
# 1. Install Docker (once). Then log out and log back in.
curl -fsSL https://get.docker.com | sudo sh
sudo usermod -aG docker "$USER"
```

```bash
# 2. Get the code and set it up
git clone https://github.com/mirosgit/reqora.git
cd reqora/server
cp .env.example .env
sed -i "s/^POSTGRES_PASSWORD=.*/POSTGRES_PASSWORD=$(openssl rand -hex 24)/" .env
sed -i "s/^REQORA_DOMAIN=.*/REQORA_DOMAIN=sync.example.com/" .env
```

```bash
# 3. Start everything: the database, the sync server, and HTTPS
docker compose up -d
```

Wait a minute (the first start downloads and builds everything, and gets an HTTPS certificate), then check:

```bash
curl https://sync.example.com/health
```

You should see `{"status":"ok","version":"0.1.0"}`. If not, see the troubleshooting table in
[server/DEPLOY.md](server/DEPLOY.md#troubleshooting).

```bash
# 4. Make an invite code for yourself (with your email)
docker compose exec server python -m reqora_server.admin create-invite --email you@example.com
```

Now do **step 5 of Option A** in Reqora, with `https://sync.example.com` as the server.

The server restarts by itself after a reboot. For backups, updates and security settings, read
[server/DEPLOY.md](server/DEPLOY.md).

### Connect your other computers

On each of your other computers:

1. Install Reqora (see [Installation](#installation)).
2. Click **Sync off** at the bottom left.
3. **Server**: the same address as before. Click **Check**.
4. Stay on the **Sign in** tab (**not** Create account), enter the same email and password. No invite code
   is needed.
5. **Upload this device's data**: leave it on to merge what's already on this computer into your account.
   Turn it off to start clean: what's on the computer is saved to a backup file and replaced with your
   account's data.
6. Click **Sign in**. Your collections show up within a few seconds.

### Invite another person

Each person gets their own account (they won't see your collections, and you won't see theirs).
Whoever runs the server makes a code with **their** email:

```bash
# Option A (in the server folder):
.venv/bin/python -m reqora_server.admin create-invite --email friend@example.com
# Option B (on the server, in reqora/server):
docker compose exec server python -m reqora_server.admin create-invite --email friend@example.com
```

Send them the code and the server address. They follow step 5 of Option A on their computer.
Treat the code like a key: send it through a channel you trust.

### Everyday things

| I want to… | Do this |
|---|---|
| Know whether sync works | Look at the bottom left. **Synced** (green) means all good. **Sync error** (red): hold the mouse over it to see why. |
| Sync right now | **Sync & account** → **Sync now** |
| Sign out a lost or stolen laptop | On any other computer: **Sync & account** → **Devices** → **Sign out** next to it. Then change your password. |
| Change my password | **Sync & account** → **Password**. This signs you out everywhere else; sign in there with the new one. |
| Reset a forgotten password | Whoever runs the server: `admin set-password you@example.com` (run it like the `create-invite` command above), then sign in with the new password. |
| Stop syncing on this computer | **Sync & account** → **Sign out here**. Your collections stay on the computer. |
| See all accounts / unused invite codes | `admin list-users` / `admin list-invites` (run like `create-invite` above) |

### When something goes wrong

| Message | What to do |
|---|---|
| "Cannot connect to the server" | The server isn't running (Option A: is the terminal from step 3 still open?), or the address is mistyped. |
| "No Reqora sync server at this address" | Something else answers at that address. Use exactly `http://127.0.0.1:8000` (or your port) or `https://sync.example.com`, with nothing after it. |
| `address already in use` when starting the server | Another program uses port 8000: start with `--port 8001` (see step 3) and use `http://127.0.0.1:8001` in Reqora. |
| `python3 -m venv` fails: "ensurepip is not available" | On Ubuntu/Debian: `sudo apt install python3-venv`, then repeat step 2. |
| "This server is invite-only" | You're on **Create account** without a code. Make one (step 4) and type it into **Invite code**. If you already have an account, use **Sign in** instead. |
| "This invite code is invalid, expired or already used" | A typo, older than 7 days, already used, or made for a different email. Make a new one. |
| "An account with this email already exists" | You already have an account: use **Sign in**. |
| "Wrong email or password" | Check both. Forgotten password: see "Reset a forgotten password" above. |
| "Too many attempts" | Wait a minute and try again. |
| "Signed out: the session expired or was revoked" | This computer was signed out (from **Devices**, or by a password change). Sign in again. |
| "1 item is too large for the server" | A request with a very big body (over 1 MB) can't be synced. Everything else still syncs. |

### More details

- Changes are sent about 2 seconds after you save them, and Reqora checks for changes from your other
  computers every 30 seconds. If you edit the same request on two computers, the edits are merged field by
  field; if both changed the same field, the computer that synced last wins.
- Theme, panel layout, the active environment and open tabs are kept per computer, not synced.
- [server/DEPLOY.md](server/DEPLOY.md): running the server on the internet properly (firewall, backups,
  updates, an existing nginx, running without Docker).
- [server/README.md](server/README.md): server settings, the HTTP API and how sync works inside.

## Where data is stored

| OS | Directory |
|---|---|
| Linux | `~/.local/share/reqora/` (or `$XDG_DATA_HOME/reqora`) |
| macOS | `~/Library/Application Support/Reqora/` |
| Windows | `%APPDATA%\Reqora\` |

Files:

- `workspace.json`: collections (with comments and variables), environments, settings
- `history.json`: the last 300 requests
- `session.json`: open tabs
- `account.json`: the sync server, account and session token (only when signed in; readable only by you on Linux/macOS)
- `sync_state.json`: what was last synced, so only changes are sent

It's plain JSON, so you can keep it in git or back it up. If a file is corrupted, the app
won't overwrite it; it renames it to `*.corrupt-<timestamp>.json` instead. The directory can be overridden
with the `REQORA_DATA_DIR` variable. On Linux, data from the old version (`~/.local/share/free-postman`)
is migrated automatically on first launch.

## Development

```bash
cargo test            # unit tests: cURL, Postman, variables, HTTP against a local server, etc.
cargo run

# Automated UI run that takes window screenshots (development only).
# The scenario modifies data, so run it against a separate directory:
REQORA_DATA_DIR=/tmp/reqora-data REQORA_SHOT_DIR=/tmp/reqora-shots cargo run --features dev-screenshot
# Add REQORA_SHOT_SYNC="<server> <email> <password>" to also sign in to a real sync server and capture the result,
# plus REQORA_SHOT_INVITE=<code> to create that account with an invite code instead.
```

Layout of `src/`:

| File | Contents |
|---|---|
| `model.rs` | data model (request, collection, environment, comment, history), URL ↔ params sync |
| `http.rs` | request preparation (variables, auth, body) and sending via reqwest |
| `response.rs` | response type detection, JSON/XML formatting, line splitting, search |
| `curl.rs`, `postman.rs` | cURL and Postman import/export |
| `vars.rs` | substitution of `{{variables}}` and `{variables}` |
| `storage.rs` | persistence to disk |
| `sync.rs` | sync with the sync server: workspace ↔ documents, push/pull, merging, the HTTP client |
| `app.rs` | application state, tabs, actions, dialogs, collection overview |
| `theme.rs`, `ui_kit.rs` | design system: palettes, fonts, buttons, toggles, chips, avatars |
| `ui_*.rs`, `widgets.rs`, `highlight.rs` | UI screens and widgets (`ui_sync.rs`: the Sync & account dialog) |

License: MIT. Inter font: SIL OFL 1.1 (`assets/fonts/LICENSE-Inter.txt`).
Icons: [Phosphor](https://phosphoricons.com/) (MIT).
