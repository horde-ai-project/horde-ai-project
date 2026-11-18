# Getting Started

One page to go from zero to a running Horde node.

Copyright (c) 2026 Adam Webber.

## What Horde is

Invite-bounded **P2P GPU compute** for AI studios. Studios call the HTTP API. Horde ships a **localhost operator UI** (Host / Nodes / Jobs / Monitor) — not an AI studio — and does not use cryptocurrency.

## 1. Install Rust

Get a stable toolchain from [https://rustup.rs](https://rustup.rs), then confirm:

```bash
cargo --version
```

## 2. Clone and build

```bash
git clone https://github.com/horde-ai-project/horde-ai-project.git
cd horde-ai-project
cargo test --workspace
```

## 3. Set `HORDE_PASSPHRASE` and run

Pick your own passphrase (required; no default). Never commit it.

**Windows (PowerShell):**

```powershell
$env:HORDE_PASSPHRASE = '<set-your-own-passphrase>'
$env:HORDE_DATA_DIR = './horde-data'
$env:HORDE_BIND = '127.0.0.1:8787'
cargo run -p horde-api
```

**Linux (bash):**

```bash
export HORDE_PASSPHRASE='<set-your-own-passphrase>'
export HORDE_DATA_DIR=./horde-data
export HORDE_BIND=127.0.0.1:8787
cargo run -p horde-api
```

Copy the studio API key from the startup log. Check liveness:

```bash
curl -s http://127.0.0.1:8787/health
```

Open the operator UI at `http://127.0.0.1:8787/ui/` and paste the API key. Or use `./scripts/horde-start.sh` / `.\scripts\horde-start.ps1` (optional browser open via `HORDE_OPEN_BROWSER=1`).

## 4. Call the API

Authenticated routes need:

`Authorization: Bearer <api_key>`

Example job (local stub):

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

Every job must pin `model_id` + `revision_hash`.

## 5. Optional: join an invite mesh

1. Set `HORDE_MESH_BIND` on each node (separate from the HTTP API bind).
2. Inviter mints `purpose: "mesh_join"` via `POST /v1/admin/invites`.
3. Joiner dials `POST /v1/admin/mesh/join` with the peer mesh address and invite token.
4. Bootstrap peers are publish-only; invite join is still required for soft trust.

Quick demo script: `./scripts/demo-two-node.sh`.

## Where next

| Doc | Use when |
|-----|----------|
| [INSTALL.md](INSTALL.md) | Full install notes (Windows + Linux), env table, mesh safety |
| [api/README.md](api/README.md) | Endpoint reference for studios and admins |
| [api/openapi.yaml](api/openapi.yaml) | Machine-readable API contract |
| [README.md](../README.md) | Project overview and Phase D features |
