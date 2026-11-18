# Operations guide

How to run each day-to-day Horde operation in plain English. This page assumes you already have the source on disk. For first-time install, see [INSTALL.md](INSTALL.md). For a short first run, see [GETTING_STARTED.md](GETTING_STARTED.md).

Copyright (c) 2026 Adam Webber.

Horde is an invite-bounded P2P GPU compute substrate for AI studios. The localhost UI is for **operators**, not an AI studio. There is **no cryptocurrency** and no spendable credits. Soft reputation is a local score among invite-trusted peers — not money.

Replace placeholders such as `<api_key>` and `<set-your-own-passphrase>` with your own values. Never put real secrets in docs or commits.

Default HTTP address in examples: `http://127.0.0.1:8787`. Authenticated calls need:

`Authorization: Bearer <api_key>`

---

## 1. Install, build, passphrase, start, stop, open the UI

### Build and test

```bash
cargo check --workspace
cargo test --workspace
```

GPU hardware is not required for stub tests.

### Set passphrase and environment

`HORDE_PASSPHRASE` is required. There is no default.

```bash
cp .env.example .env
# edit .env — set HORDE_PASSPHRASE=<set-your-own-passphrase>
```

Or export in the shell:

```bash
export HORDE_PASSPHRASE='<set-your-own-passphrase>'
export HORDE_DATA_DIR=./horde-data
export HORDE_BIND=127.0.0.1:8787
```

```powershell
$env:HORDE_PASSPHRASE = '<set-your-own-passphrase>'
$env:HORDE_DATA_DIR = './horde-data'
$env:HORDE_BIND = '127.0.0.1:8787'
```

Use the **same** passphrase whenever you reopen an existing data directory. The passphrase is never accepted over HTTP.

Useful optional vars (full table in [INSTALL.md](INSTALL.md)):

| Variable | Role |
|----------|------|
| `HORDE_BIND` | HTTP API + UI (default `127.0.0.1:8787`) |
| `HORDE_MESH_BIND` | **Startup** mesh listen address (optional) |
| `HORDE_OPEN_BROWSER` | `1` opens the UI when using start scripts |
| `HORDE_UI_DIR` | Override path to `ui/` assets |

### Start and stop

Prefer the host scripts (they load `.env`, write `horde.pid` / `horde.log`):

```bash
./scripts/horde-start.sh
# optional: HORDE_OPEN_BROWSER=1 ./scripts/horde-start.sh
./scripts/horde-stop.sh
```

```powershell
.\scripts\horde-start.ps1
# optional: .\scripts\horde-start.ps1 -OpenBrowser
.\scripts\horde-stop.ps1
```

Or run in the foreground:

```bash
cargo run -p horde-api
```

The browser **cannot** start or stop the binary. Passphrase stays on the host (env / `.env`).

### Open the operator UI

1. Copy the studio API key from the startup log (`horde.log` or console).
2. Open `http://127.0.0.1:8787/ui/` (root `/` redirects there).
3. Paste the key in the top bar → **Save**.

Quick check without the UI:

```bash
curl -s http://127.0.0.1:8787/health
curl -s http://127.0.0.1:8787/v1/meta
```

Four panels: **Host**, **Nodes**, **Jobs**, **Monitor**.

---

## 2. Host panel

### Status

Open **Host** and click **Refresh** (or wait for the online pill).

You see health, mode (local vs mesh), node ID, member/job counts, HTTP bind, data directory, and whether mesh is enabled.

API equivalents:

```bash
curl -s http://127.0.0.1:8787/health
curl -s http://127.0.0.1:8787/v1/admin/status -H "Authorization: Bearer <api_key>"
curl -s http://127.0.0.1:8787/v1/admin/config -H "Authorization: Bearer <api_key>"
```

`GET /v1/admin/config` is redacted: it shows bind / data dir / mesh flags and **never** the passphrase.

### Server limits

Under **Server limits**, pick a server, set GPU %, time window hours, disk budget (MiB), and whether the server is enabled / accepts network jobs → **Save limits**.

```bash
curl -s -X POST http://127.0.0.1:8787/v1/admin/servers/<server_id>/limits \
  -H "Authorization: Bearer <api_key>" \
  -H 'Content-Type: application/json' \
  -d '{
    "gpu_percent": 50,
    "time_window": { "start_hour": 22, "end_hour": 7, "max_gpu_minutes_per_day": 240 },
    "disk_budget_mb": 50000,
    "accept_network_jobs": true,
    "enabled": true
  }'
```

### Mesh bind (runtime change)

Mesh uses a **separate** address from the HTTP API. You can set it at startup **or** while running (no process restart).

**At start:** set `HORDE_MESH_BIND` (for example `127.0.0.1:9797`). Omit it to stay local-only until you enable mesh later.

**While running:**

1. **Host → Mesh bind** — enter `host:port` (for example `127.0.0.1:0` or `0.0.0.0:9100`) → **Apply bind**.
2. Or **Nodes → Set bind** the same way.

```bash
curl -s -X POST http://127.0.0.1:8787/v1/admin/mesh/bind \
  -H "Authorization: Bearer <api_key>" \
  -H 'Content-Type: application/json' \
  -d '{"bind":"127.0.0.1:0"}'
```

That is a true in-process rebind. Port `0` asks the OS for a free port; the response shows the real address. Existing peer/trust/blocklist state is kept. `HORDE_MESH_BIND` remains only the startup default for the next process start.

### Theme

**Host → Theme** — pick a stylesheet (coffee, business blue/white, and others). Choice is stored in the browser only.

### Extra API key

**Host → Extra API key** — enter a label → **Mint key**. Copy the new key when shown; treat it like a password.

```bash
curl -s -X POST http://127.0.0.1:8787/v1/admin/api-keys \
  -H "Authorization: Bearer <api_key>" \
  -H 'Content-Type: application/json' \
  -d '{"label":"operator"}'
```

---

## 3. Nodes panel (invite mesh)

Mesh is invite-bounded by default. Bootstrap peers are **publish-only** (they do not grant job trust). Soft trust still requires an invite join. Local blocklists always win over soft reputation.

### Mesh status

**Nodes → Refresh mesh** (or open the panel with a saved API key).

Shows bind, peers, bootstrap list, blocklist, and DHT flag.

```bash
curl -s http://127.0.0.1:8787/v1/admin/mesh/status \
  -H "Authorization: Bearer <api_key>"
```

### Mint an invite

**Nodes → Mint invite**

1. Purpose: usually `mesh_join` (or `collective_join` for collective bookkeeping).
2. TTL in seconds (default 3600).
3. Optional note → **Mint**.
4. Copy the plaintext token **once** — it is not shown again.

```bash
curl -s -X POST http://127.0.0.1:8787/v1/admin/invites \
  -H "Authorization: Bearer <api_key>" \
  -H 'Content-Type: application/json' \
  -d '{"purpose":"mesh_join","ttl_secs":3600,"note":"lab peer"}'
```

### Join with an invite

On the joining node (**Nodes → Join with invite**):

1. Peer mesh address (the inviter’s mesh bind, not the HTTP bind).
2. Invite token → **Join**.

```bash
curl -s -X POST http://127.0.0.1:8787/v1/admin/mesh/join \
  -H "Authorization: Bearer <api_key>" \
  -H 'Content-Type: application/json' \
  -d '{"peer_addr":"127.0.0.1:9797","token":"<invite_token>"}'
```

Two-node loopback demo: `./scripts/demo-two-node.sh`.

### Bootstrap peers

**Nodes → Bootstrap peers** — comma-separated addresses → **Set bootstrap**.

This only replaces the publish-only dial list. It does **not** replace invite join.

```bash
curl -s -X POST http://127.0.0.1:8787/v1/admin/mesh/bootstrap \
  -H "Authorization: Bearer <api_key>" \
  -H 'Content-Type: application/json' \
  -d '{"peers":[{"addr":"127.0.0.1:9797","note":"seed"}]}'
```

### Blocklist

**Nodes → Blocklist** — enter a node ID → **Block** or **Unblock**.

Blocked peers stay denied even if soft scores look fine.

```bash
curl -s -X POST http://127.0.0.1:8787/v1/admin/mesh/blocklist \
  -H "Authorization: Bearer <api_key>" \
  -H 'Content-Type: application/json' \
  -d '{"node_id":"<node_id>"}'

curl -s -X DELETE http://127.0.0.1:8787/v1/admin/mesh/blocklist/<node_id> \
  -H "Authorization: Bearer <api_key>"
```

### Public DHT stub (advanced, default off)

Under **Advanced: public DHT stub**, check enable → **Apply**. Or set `HORDE_PUBLIC_DHT=1` before start. The stub does not discover public peers; invite join remains the normal path.

```bash
curl -s -X POST http://127.0.0.1:8787/v1/admin/mesh/dht \
  -H "Authorization: Bearer <api_key>" \
  -H 'Content-Type: application/json' \
  -d '{"enabled":true}'
```

---

## 4. Jobs panel

The Jobs panel is a thin submit/poll tool for operators and integrators — not a full AI studio.

Every job must pin `model_id` and `revision_hash`. For `tokenize` / `embed`, also pin `tokenizer_revision`.

### Submit

**Jobs → Submit**

1. Choose kind (`stub_echo`, `tokenize`, `text_generate`, `embed`, `video_generate`).
2. Routing: `local` (this node), `mesh` (invite peers), or `auto` (local first, else mesh).
3. Fill fingerprint fields and payload → **Submit job**.
4. The new job ID is filled into **Track job**.

```bash
curl -s -X POST http://127.0.0.1:8787/v1/jobs \
  -H "Authorization: Bearer <api_key>" \
  -H 'Content-Type: application/json' \
  -d '{
    "fingerprint": {
      "model_id": "stub/echo",
      "revision_hash": "deadbeefcafebabe",
      "precision": "fp16"
    },
    "kind": "stub_echo",
    "payload": "hello horde",
    "routing": "local"
  }'
```

Optional soft `priority_hint` (0–100) mildly prefers reciprocal mesh peers. It is **not** a paid queue jump.

### Poll, cancel, result, accept, reject

**Jobs → Track job** — paste a job ID:

| Button | What it does |
|--------|----------------|
| **Poll** | Latest status |
| **Cancel** | Stop if not already finished |
| **Result** | Output + worker attestation when succeeded |
| **Accept** | You accept the attested result (updates soft reputation for mesh peers) |
| **Reject** | You reject it (same — local soft scores only) |

```bash
curl -s http://127.0.0.1:8787/v1/jobs/<job_id> \
  -H "Authorization: Bearer <api_key>"

curl -s http://127.0.0.1:8787/v1/jobs/<job_id>/result \
  -H "Authorization: Bearer <api_key>"

curl -s -X POST http://127.0.0.1:8787/v1/jobs/<job_id>/cancel \
  -H "Authorization: Bearer <api_key>"

curl -s -X POST http://127.0.0.1:8787/v1/jobs/<job_id>/accept \
  -H "Authorization: Bearer <api_key>"

curl -s -X POST http://127.0.0.1:8787/v1/jobs/<job_id>/reject \
  -H "Authorization: Bearer <api_key>"
```

Attestations are for audit, not payment.

### Job history

**Refresh list** loads recent jobs from the server. This browser also remembers recent IDs locally; **Clear local history** drops only that browser list.

```bash
curl -s 'http://127.0.0.1:8787/v1/jobs?limit=50' \
  -H "Authorization: Bearer <api_key>"
```

---

## 5. Monitor panel

**Monitor** is a live snapshot of collective health. It refreshes while the panel is open.

### What you see

- Counts: members online, job count, mesh peers, mode
- **Members** table: server ID, host, GPU %, enabled, network jobs
- **Soft reputation**: local per-peer scores and fair-share settings — **not currency**, not transferable, not a global ban list
- **Recent jobs**: id, kind, status, routing, updated time

```bash
curl -s http://127.0.0.1:8787/v1/admin/status -H "Authorization: Bearer <api_key>"
curl -s http://127.0.0.1:8787/v1/admin/reputation -H "Authorization: Bearer <api_key>"
curl -s 'http://127.0.0.1:8787/v1/jobs?limit=30' -H "Authorization: Bearer <api_key>"
```

Optional: set a per-peer priority hint (0–100) among invite-trusted peers:

```bash
curl -s -X POST http://127.0.0.1:8787/v1/admin/reputation/<node_id>/priority \
  -H "Authorization: Bearer <api_key>" \
  -H 'Content-Type: application/json' \
  -d '{"priority_hint":80}'
```

Local mode (no mesh) does not apply soft-reputation effects between peers.

---

## Safety reminders

- Keep the admin HTTP API on loopback unless you know what you are doing.
- Treat `HORDE_PASSPHRASE`, API keys, and invite plaintext as secrets.
- Soft reputation never replaces your local blocklist.
- There is no cryptocurrency, convertible credit, or paid queue in Horde.

## Related docs

| Doc | Use when |
|-----|----------|
| [INSTALL.md](INSTALL.md) | Full install (Windows + Linux), env table |
| [GETTING_STARTED.md](GETTING_STARTED.md) | Short path from zero to first call |
| [api/README.md](api/README.md) | Endpoint reference |
| [api/openapi.yaml](api/openapi.yaml) | Machine-readable contract |
| [README.md](../README.md) | Project overview |

## License

Horde is licensed under the [PolyForm Noncommercial License 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0). See [`LICENSE`](../LICENSE).
