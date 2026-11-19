# Horde Studio & Admin API (Phase D)

Base URL (default): `http://127.0.0.1:8787`

Auth: `Authorization: Bearer <api_key>` on all `/v1/*` routes except `/health` and `/v1/meta`.

Machine-readable contract: [`openapi.yaml`](./openapi.yaml).

Admin/studio HTTP is **localhost / collective-only**. The invite-bounded mesh listens on a **separate** bind and never carries admin traffic. Public DHT discovery is an **explicit opt-in stub** (default off). Soft reputation is local and non-transferable.

**Mesh bind:** env `HORDE_MESH_BIND` is the **startup** default. While running, enable or rebind with `POST /v1/admin/mesh/bind` (true runtime rebind — no process restart) or the operator UI (**Host → Mesh bind** / **Nodes → Set bind**). `GET /v1/admin/config` returns the current redacted bind (`mesh_bind`, `mesh_enabled`) and **never** the passphrase.

## Concepts

| Term | Meaning |
|------|---------|
| **Collective** | Logical node: one peer identity, many physical servers |
| **Physical server** | One OS host with GPU caps + time window |
| **Model fingerprint** | Required `model_id` + `revision_hash` (+ tokenizer/precision) |
| **Tokenise chunking** | Shardable pipeline under **one** pinned fingerprint + seed |
| **Support matrix** | Per-kind backends, VRAM floors, shardability, video gates |
| **Invite** | Short-lived token for collective join or mesh soft-trust |
| **Mesh** | Noise-encrypted peer channels among invited nodes |
| **Soft reputation** | Local, non-transferable per-peer scores (Phase D) |
| **Fair-share** | Prefer reciprocal peers; ignore pure consumers after threshold |
| **Priority hint** | Optional 0–100 soft hint among invite-trusted peers (not credits) |
| **Public DHT** | Optional opt-in stub; rate-limited + resource-proof hooks |
| **Bootstrap peers** | Static/config publish-only dial list — **not** a job authority |
| **Blocklist** | Local operator deny list (first-class; overrides soft reputation) |
| **Mute list** | Soft pause accepting inbound jobs from a peer (invite mesh kept) |

## Endpoints

### Public

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Liveness |
| GET | `/v1/meta` | Product/mode/node id (+ mesh + `public_dht_enabled` + `soft_reputation`) |
| GET | `/` → `/ui/` | Operator UI redirect (static assets under `/ui/*`) |
| GET | `/ui/help/` | Bundled operations guide (`OPERATIONS.md`) |

### Admin (localhost / collective)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/admin/status` | Collective overview + members (+ mesh status) |
| GET | `/v1/admin/config` | Redacted runtime config (HTTP bind, data_dir, live `mesh_bind` / `mesh_enabled` — **never** passphrase) |
| GET | `/v1/admin/servers` | List physical servers |
| POST | `/v1/admin/servers/{server_id}/limits` | Set GPU %, time window, disk, flags |
| GET | `/v1/admin/support-matrix` | Workload gates (backends / VRAM / shardable) |
| GET | `/v1/admin/invites` | List invite records (hashed secrets) |
| POST | `/v1/admin/invites` | Mint invite (plaintext returned once) |
| POST | `/v1/admin/invites/redeem` | Redeem invite (collective join bookkeeping) |
| POST | `/v1/admin/api-keys` | Create an additional studio API key |
| GET\|POST | `/v1/admin/drain` | Graceful drain status / begin drain (finish in-flight, then exit) |
| POST | `/v1/admin/diagnostics/join` | Pre-connect checks (bind, invite, clock skew, API key) |
| GET | `/v1/admin/export` | Local-only JSON snapshot of jobs + peer stats |
| GET | `/v1/admin/mesh/status` | Mesh bind, peers, bootstrap, blocklist, DHT flag |
| GET | `/v1/admin/mesh/health` | Per-peer last-seen, RTT, accept/reject rates (read-only) |
| POST | `/v1/admin/mesh/bind` | Enable or **runtime-rebind** mesh listen address (`{ "bind": "host:port" }`; no process restart; never passphrase) |
| POST | `/v1/admin/mesh/bootstrap` | Replace publish-only bootstrap peer list |
| POST | `/v1/admin/mesh/join` | Dial peer with mesh invite token |
| GET\|POST | `/v1/admin/mesh/blocklist` | List / block |
| DELETE | `/v1/admin/mesh/blocklist/{node_id}` | Unblock |
| GET\|POST | `/v1/admin/mesh/mute` | List / soft-mute (pause accept jobs; not a kick) |
| DELETE | `/v1/admin/mesh/mute/{node_id}` | Unmute |
| GET\|POST | `/v1/admin/mesh/dht` | DHT stub status / explicit opt-in config |
| POST | `/v1/admin/mesh/dht/challenge` | Issue resource-proof challenge |
| POST | `/v1/admin/mesh/dht/lookup` | Rate-limited stub lookup |
| GET\|POST | `/v1/admin/reputation` | Soft reputation snapshot / fair-share config |
| POST | `/v1/admin/reputation/{node_id}/priority` | Set per-peer priority hint (0–100) |

### Studio (AI product integration)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/jobs` | List recent jobs (`?limit=`, default 50, max 500) |
| POST | `/v1/jobs` | Create job → local and/or mesh schedule |
| GET | `/v1/jobs/{job_id}` | Job status (includes timeline) |
| GET | `/v1/jobs/{job_id}/timeline` | State history with timestamps and peer/server |
| GET | `/v1/jobs/{job_id}/result` | Result + worker attestation (when succeeded) |
| POST | `/v1/jobs/{job_id}/cancel` | Cancel if not terminal |
| POST | `/v1/jobs/{job_id}/accept` | Requester accepts attested result (updates soft reputation) |
| POST | `/v1/jobs/{job_id}/reject` | Requester rejects attested result (updates soft reputation) |

## Job create body

```json
{
  "fingerprint": {
    "model_id": "stub/tokenizer",
    "revision_hash": "deadbeefcafebabe",
    "tokenizer_revision": "tok-rev-1",
    "precision": "fp16"
  },
  "kind": "tokenize",
  "payload": "{\"documents\":[\"a b\",\"c d e\"],\"chunk_size\":1,\"seed\":7}",
  "requested_vram_mb": null,
  "routing": "local",
  "priority_hint": 70
}
```

`routing` values: `local` (default), `mesh`, `auto` (local first, else mesh).

`priority_hint` (optional, 0–100): soft preference among invite-trusted mesh peers. Not a credit spend.

`kind` values:

| Kind | Shardable | Notes |
|------|-----------|-------|
| `stub_echo` | no | CI smoke |
| `text_generate` | no | Whole-unit generative |
| `tokenize` | **yes** | Requires `tokenizer_revision`; deterministic chunks |
| `embed` | **yes** | Requires `tokenizer_revision`; stub checksum path |
| `video_generate` | no | Hard gate: CUDA/ROCm + ≥12288 MiB; stub rejects |

## Mesh bind body

Enable mesh if it was off, or rebind the accept socket **in-process** (no restart). Overrides the startup `HORDE_MESH_BIND` for the running process. Same action as Host → Mesh bind / Nodes → Set bind in `/ui/`.

```bash
curl -s -X POST http://127.0.0.1:8787/v1/admin/mesh/bind \
  -H "Authorization: Bearer <api_key>" \
  -H 'Content-Type: application/json' \
  -d '{"bind":"127.0.0.1:0"}'
```

```json
{ "bind": "127.0.0.1:0" }
```

Successful response includes the actual listen address (ephemeral `:0` is resolved), for example:

```json
{
  "ok": true,
  "mesh_bind": "127.0.0.1:54321",
  "mesh_enabled": true,
  "rebind": "runtime"
}
```

Then `GET /v1/admin/config` reflects the same `mesh_bind` / `mesh_enabled` values. Passphrase is never accepted on this route and never returned from config.

## Soft reputation body

`GET /v1/admin/reputation` returns peers + fair-share config.

`POST /v1/admin/reputation` updates fair-share thresholds:

```json
{
  "consumer_ratio_threshold": 5.0,
  "min_samples_before_ignore": 3
}
```

`POST /v1/admin/reputation/{node_id}/priority`:

```json
{ "priority_hint": 80 }
```

## Limits body

```json
{
  "gpu_percent": 50,
  "time_window": {
    "start_hour": 22,
    "end_hour": 7,
    "max_gpu_minutes_per_day": 240
  },
  "disk_budget_mb": 50000,
  "accept_network_jobs": true,
  "enabled": true
}
```

## Public DHT (opt-in)

```json
{
  "enabled": true,
  "max_queries_per_minute": 30,
  "require_resource_proof": true
}
```

Env: `HORDE_PUBLIC_DHT=1` (optional `HORDE_DHT_QPM`). Invite join remains the default; the stub does not discover public peers.

## Errors

JSON shape: `{ "error": "<message>" }` with appropriate HTTP status (`401`, `404`, `400`, `429`, `500`).

## Safety notes for integrators

- Pin fingerprints on every job; do not send arbitrary model URLs.
- Tokenise/embed must pin `tokenizer_revision`; chunk only under that pin.
- Treat invite plaintext and API keys as secrets.
- Do not expose the admin API on the public internet; defaults to loopback.
- Mesh is invite-bounded by default; public DHT is opt-in only.
- Result attestations are for audit, not payment. There is no cryptocurrency.
- Soft reputation is local and non-transferable; never a credits ledger.
- Local blocklists override soft trust; gossip is not an authority.
