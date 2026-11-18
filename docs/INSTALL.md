# Install & setup

Plain-language steps to build and run Horde on your machine. Horde is an invite-bounded P2P GPU compute substrate for AI studios. There is no built-in studio UI and no cryptocurrency.

Copyright (c) 2026 Adam Webber.

## What you need

- **Rust toolchain** (stable). Install from [https://rustup.rs](https://rustup.rs).
- **Git** (to clone the repo).
- A shell: PowerShell on Windows, or bash on Linux.
- Optional: `curl` and `jq` if you want to follow the two-node mesh demo script.

GPU hardware is **not** required for install, build, or stub tests.

## Get the source

```bash
git clone https://github.com/horde-ai-project/horde-ai-project.git
cd horde-ai-project
```

On the author’s Windows machine the working tree often lives at `C:\xampp\htdocs\horde`. Any checkout path is fine.

## Install Rust (if needed)

### Windows

1. Download and run `rustup-init.exe` from [https://rustup.rs](https://rustup.rs).
2. Prefer the default stable toolchain when prompted.
3. Open a **new** PowerShell window, then check:

```powershell
rustc --version
cargo --version
```

If `cargo` is missing, ensure `%USERPROFILE%\.cargo\bin` is on your `PATH`.

### Linux

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustc --version
cargo --version
```

On some distros you may also need a C linker and build tools (for example `build-essential` on Debian/Ubuntu, or the equivalent “Development Tools” group on RHEL-family systems).

## Build & test

From the repo root:

```bash
cargo check --workspace
cargo test --workspace
```

Windows (PowerShell) is the same:

```powershell
cargo check --workspace
cargo test --workspace
```

## Environment

`HORDE_PASSPHRASE` is **required**. There is no embedded default. Choose your own strong passphrase; never commit real secrets.

Copy the example file and fill it in locally (`.env` is gitignored):

```bash
cp .env.example .env
```

Useful variables (see [`.env.example`](../.env.example)):

| Variable | Required | Meaning |
|----------|----------|---------|
| `HORDE_PASSPHRASE` | **yes** | Unlocks / creates the local encrypted keystore |
| `HORDE_DATA_DIR` | no | Data directory (default `./horde-data`) |
| `HORDE_HOSTNAME` | no | Friendly host label |
| `HORDE_BIND` | no | HTTP API bind (default `127.0.0.1:8787`) |
| `HORDE_MESH_BIND` | no | Mesh listener at start; omit for local-only (can also enable/rebind at runtime via `POST /v1/admin/mesh/bind` or Host/Nodes in `/ui/`) |
| `HORDE_BOOTSTRAP_PEERS` | no | Comma-separated publish-only peer addresses |
| `HORDE_PUBLIC_DHT` | no | Set `1` only to opt into the public DHT stub (default off) |
| `HORDE_UI_DIR` | no | Path to operator UI assets (default: resolve `./ui`) |
| `HORDE_OPEN_BROWSER` | no | `1` / `true` opens the UI when using start scripts |

The `horde` binary does not auto-load `.env`. Export variables in your shell, or use `scripts/horde-start.*` which source `.env` when present.

### Set the passphrase

**Windows (PowerShell):**

```powershell
$env:HORDE_PASSPHRASE = '<set-your-own-passphrase>'
$env:HORDE_DATA_DIR = './horde-data'
$env:HORDE_BIND = '127.0.0.1:8787'
```

**Linux (bash):**

```bash
export HORDE_PASSPHRASE='<set-your-own-passphrase>'
export HORDE_DATA_DIR=./horde-data
export HORDE_BIND=127.0.0.1:8787
```

Use the **same** passphrase every time you reopen an existing `HORDE_DATA_DIR`, or the keystore will not unlock.

## Run the API

```bash
cargo run -p horde-api
```

Or use the host launcher scripts (load `.env`, write `horde.pid` / `horde.log`):

```bash
./scripts/horde-start.sh
./scripts/horde-stop.sh
```

```powershell
.\scripts\horde-start.ps1
.\scripts\horde-stop.ps1
```

Set `HORDE_OPEN_BROWSER=1` (or `-OpenBrowser` on PowerShell) to open the operator UI automatically.

On startup the log prints a studio API key. Use it as:

`Authorization: Bearer <api_key>`

on `/v1/*` routes (except `/health` and `/v1/meta`). Keep that key private; do not paste real keys into docs or commits.

Quick check:

```bash
curl -s http://127.0.0.1:8787/health
curl -s http://127.0.0.1:8787/v1/meta
```

### Operator UI

Static assets under `ui/` are served by `horde-api` at **`http://127.0.0.1:8787/ui/`** (root `/` redirects there). Paste the API key into the top bar. Panels: Host, Nodes, Jobs, Monitor. This is operator control — not an AI studio. Start/stop still uses the scripts above (the browser cannot spawn the binary). Optional `HORDE_UI_DIR` overrides the asset path. Mesh bind can be set from Host or Nodes (`POST /v1/admin/mesh/bind`) without restarting the process.

Without `HORDE_MESH_BIND` (and before a runtime bind), the node runs in **local** mode (no mesh reputation effects).

## Invite mesh basics (optional)

Mesh traffic uses a **separate** bind from the HTTP admin/studio API.

1. Start each node with its own data dir, passphrase, API bind, and mesh bind.
2. On the inviter, mint a short-lived invite with purpose `mesh_join` via `POST /v1/admin/invites`.
3. On the joiner, dial with `POST /v1/admin/mesh/join` and body `{ "peer_addr": "<inviter-mesh-addr>", "token": "<invite-token>" }`.
4. `HORDE_BOOTSTRAP_PEERS` is publish-only — it does **not** grant job authority. Soft trust still requires an invite join.
5. Public DHT discovery stays **off** unless you explicitly opt in (`HORDE_PUBLIC_DHT=1` or `POST /v1/admin/mesh/dht`).

Automated two-node loopback demo (Linux/macOS bash, or Git Bash / WSL on Windows):

```bash
./scripts/demo-two-node.sh
```

## Safety reminders

- Do not expose the admin HTTP API on the public internet; defaults are loopback.
- Treat invite plaintext, API keys, and `HORDE_PASSPHRASE` as secrets.
- Soft reputation is local and non-transferable — not credits or currency.
- Local blocklists override soft trust.

## Next steps

- One-page overview: [GETTING_STARTED.md](GETTING_STARTED.md)
- Human API guide: [api/README.md](api/README.md)
- OpenAPI contract: [api/openapi.yaml](api/openapi.yaml)

## License

Horde is licensed under the [PolyForm Noncommercial License 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0). See [`LICENSE`](../LICENSE).
