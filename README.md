# Reqora

A fast, lightweight open-source API client (a Postman alternative) for Linux, Windows and macOS,
written in Rust: the GUI is built with [egui](https://github.com/emilk/egui) (GPU-rendered), HTTP runs on
`reqwest` + `tokio`. It ships as a single binary and needs no system `-dev` packages. The Inter font and
Phosphor icons are bundled, so the interface looks the same on every machine.

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
| `app.rs` | application state, tabs, actions, dialogs, collection overview |
| `theme.rs`, `ui_kit.rs` | design system: palettes, fonts, buttons, toggles, chips, avatars |
| `ui_*.rs`, `widgets.rs`, `highlight.rs` | UI screens and widgets |

License: MIT. Inter font: SIL OFL 1.1 (`assets/fonts/LICENSE-Inter.txt`).
Icons: [Phosphor](https://phosphoricons.com/) (MIT).
