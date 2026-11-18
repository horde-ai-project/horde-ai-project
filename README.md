# Horde (Phase D)

Invite-bounded **P2P GPU compute substrate** for AI studios. No built-in AI studio UI. No cryptocurrency.

Phase D adds **soft, non-transferable local reputation** and **fair-share weighting** among invite-trusted peers, plus optional **priority hints** (0-100). Scores stay on the local node; they are not credits, not transferable, and never replace local blocklists. Avatar/TTS remain out of scope.

Earlier phases: invite-bounded Noise mesh (B); tokenisation chunking + support matrix + opt-in public DHT stub (C).

## Workspace

| Crate | Path | Role |
|-------|------|------|
| `horde-protocol` | `crates/horde-protocol` | Jobs, fingerprints, tokenise, DHT stub, soft reputation |
| `horde-node` | `crates/horde-node` | Physical server agent + keystore + stub executor |
| `horde-mesh` | `crates/horde-mesh` | Noise XX peer channels, invite gate, fair-share peer pick |
| `horde-collective` | `crates/horde-collective` | Logical node head, scheduling, reputation ledger |
| `horde-api` | `crates/horde-api` | HTTP API binary `horde` + operator UI at `/ui` |
| static UI | `ui/` | Localhost operator panels (Host / Nodes / Jobs / Monitor) |

## Operator UI

Horde ships a **localhost browser operator UI** (not an AI studio). After the API is up, open `http://127.0.0.1:8787/ui/` and paste the studio API key from the startup log.

Four panels: **Host** (status + limits + theme), **Nodes** (invite mesh), **Jobs** (thin submit/poll), **Monitor** (health + soft reputation).

Start/stop the process with host scripts (they read `.env`; they do not accept a passphrase over HTTP):

```bash
# Unix
cp .env.example .env   # set HORDE_PASSPHRASE
./scripts/horde-start.sh
# optional: HORDE_OPEN_BROWSER=1 ./scripts/horde-start.sh
./scripts/horde-stop.sh
```

```powershell
# Windows
Copy-Item .env.example .env   # set HORDE_PASSPHRASE
.\scripts\horde-start.ps1
# optional: .\scripts\horde-start.ps1 -OpenBrowser
.\scripts\horde-stop.ps1
```

Stylesheets: Host panel theme picker (coffee, business blue/white, and others). No Tauri shell in MVP.

## Build & test

```powershell
cd C:\xampp\htdocs\horde
cargo check --workspace
cargo test --workspace
```

GPU hardware is **not** required. Stub executor tests always run.

## Run (local mode)

`HORDE_PASSPHRASE` is **required** (no embedded default). Use your own secret; do not commit real values.

```powershell
$env:HORDE_PASSPHRASE='<set-your-own-passphrase>'
$env:HORDE_DATA_DIR='./horde-data'
$env:HORDE_BIND='127.0.0.1:8787'
cargo run -p horde-api
```

See [`.env.example`](.env.example) for variable names (copy to `.env` locally; `.env` is gitignored).

Local mode does **not** earn/spend soft reputation effects.

## Soft reputation & fair-share (Phase D)

Among invite-trusted mesh peers:

- Prefer peers who recently completed **accepted** work for you.
- Ignore pure consumers after a local threshold (configurable).
- Operator may set a per-peer `priority_hint` (0-100).
- Job create may include an optional soft `priority_hint` that mildly amplifies preference for reciprocal peers — **not** a paid queue jump.

```powershell
# Snapshot local soft scores
curl -s http://127.0.0.1:8787/v1/admin/reputation -H "Authorization: Bearer $HORDE_API_KEY"

# Set priority hint for a trusted peer
curl -s -X POST http://127.0.0.1:8787/v1/admin/reputation/<node_id>/priority `
  -H "Authorization: Bearer $HORDE_API_KEY" `
  -H 'Content-Type: application/json' `
  -d '{"priority_hint":80}'
```

## Run two nodes (invite mesh — default)

Set `HORDE_MESH_BIND` on each node at start, or enable/rebind at runtime via Host/Nodes in `/ui/` (`POST /v1/admin/mesh/bind`). Mint a `mesh_join` invite, dial with `/v1/admin/mesh/join`. Bootstrap peers (`HORDE_BOOTSTRAP_PEERS`) are publish-only.

See `docs/api/README.md` and `scripts/demo-two-node.sh`.

## Tokenisation job (Phase C)

Pin `tokenizer_revision` and submit `kind: "tokenize"` with a JSON payload. Same input + seed + fingerprint → same chunk plan and pipeline checksum.

## Optional public DHT (explicit opt-in)

Default: **disabled** (invite-only). Env `HORDE_PUBLIC_DHT=1` or `POST /v1/admin/mesh/dht`. Stub returns empty peers; invite join remains the default.

## Studio integration

AI studios call the HTTP API. The `/ui` operator shell is for node operators only — it is **not** a built-in AI studio.

Every job **must** pin a `model_id` + `revision_hash`. Results include a worker Ed25519 attestation (audit, not payment). Accept/reject updates soft reputation for mesh peers only.

## Documentation

- Install & setup: [`docs/INSTALL.md`](docs/INSTALL.md)
- Getting started: [`docs/GETTING_STARTED.md`](docs/GETTING_STARTED.md)
- Human API guide: [`docs/api/README.md`](docs/api/README.md)
- OpenAPI 3: [`docs/api/openapi.yaml`](docs/api/openapi.yaml)

## Explicitly out of scope (Phase D)

- Cryptocurrency, spendable credits, convert, merchandise
- Avatar / TTS
- Built-in AI studio panels
- Soft reputation as a global ban authority (local blocklists win)

## License

Copyright (c) 2026 Adam Webber.

Horde is licensed under the [PolyForm Noncommercial License 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0). See [`LICENSE`](LICENSE) for the full terms.