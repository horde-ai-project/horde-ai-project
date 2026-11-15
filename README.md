# Horde (Phase C)

Invite-bounded **P2P GPU compute substrate** for AI studios. No built-in AI studio UI. No cryptocurrency.

Phase C adds **tokenisation pipelines with deterministic chunking** under pinned model fingerprints, a **support matrix / resource gates** (video only behind hard VRAM gates), and an **optional public DHT discovery stub** that stays **off by default** (invite-only remains the join path). Enabling public DHT requires explicit opt-in plus rate limits and resource-proof hooks.

## Workspace

| Crate | Path | Role |
|-------|------|------|
| `horde-protocol` | `crates/horde-protocol` | Jobs, fingerprints, tokenise chunking, DHT stub, support matrix |
| `horde-node` | `crates/horde-node` | Physical server agent + keystore + stub executor |
| `horde-mesh` | `crates/horde-mesh` | Noise XX peer channels, invite gate, job RPC, DHT opt-in stub |
| `horde-collective` | `crates/horde-collective` | Logical node head, scheduling, mesh host hooks |
| `horde-api` | `crates/horde-api` | HTTP API binary `horde` |

## Build & test

```powershell
cd C:\xampp\htdocs\horde
cargo check --workspace
cargo test --workspace
```

GPU hardware is **not** required. Stub executor tests always run.

## Run (local mode)

```powershell
$env:HORDE_PASSPHRASE='choose-a-strong-passphrase'
$env:HORDE_DATA_DIR='./horde-data'
$env:HORDE_BIND='127.0.0.1:8787'
cargo run -p horde-api
```

## Run two nodes (invite mesh — default)

Same as Phase B: set `HORDE_MESH_BIND` on each node, mint a `mesh_join` invite, dial with `/v1/admin/mesh/join`. Bootstrap peers (`HORDE_BOOTSTRAP_PEERS`) are publish-only.

See `docs/api/README.md` and `scripts/demo-two-node.sh`.

## Tokenisation job (Phase C)

Pin `tokenizer_revision` and submit `kind: "tokenize"` with a JSON payload:

```json
{
  "fingerprint": {
    "model_id": "stub/tokenizer",
    "revision_hash": "deadbeefcafebabe",
    "tokenizer_revision": "tok-rev-1",
    "precision": "fp16"
  },
  "kind": "tokenize",
  "payload": "{\"documents\":[\"hello world\",\"foo bar\"],\"chunk_size\":1,\"seed\":7}",
  "routing": "local"
}
```

Same input + seed + fingerprint → same chunk plan and pipeline checksum. Generative kinds (`text_generate`, `stub_echo`, `video_generate`) stay whole-unit.

## Optional public DHT (explicit opt-in)

Default: **disabled** (invite-only).

```powershell
# env opt-in (still rate-limited + proof-gated)
$env:HORDE_PUBLIC_DHT='1'
$env:HORDE_MESH_BIND='127.0.0.1:9797'
```

Or via admin API after mesh is up:

```powershell
curl -s -X POST http://127.0.0.1:8787/v1/admin/mesh/dht `
  -H "Authorization: Bearer $HORDE_API_KEY" `
  -H 'Content-Type: application/json' `
  -d '{"enabled":true,"max_queries_per_minute":30,"require_resource_proof":true}'
```

Then `POST /v1/admin/mesh/dht/challenge` → solve proof → `POST /v1/admin/mesh/dht/lookup`. The stub returns an empty peer list; it does not replace invite join.

## Studio integration

AI studios call the HTTP API; they do **not** embed Horde UI.

Every job **must** pin a `model_id` + `revision_hash`. Tokenise/embed also require `tokenizer_revision`. Wrong fingerprints are rejected. Results include a worker Ed25519 attestation (audit, not payment).

`GET /v1/admin/support-matrix` lists backends, VRAM floors, and shardability per work kind.

## Documentation

- Human API guide: [`docs/api/README.md`](docs/api/README.md)
- OpenAPI 3: [`docs/api/openapi.yaml`](docs/api/openapi.yaml)

## Explicitly out of scope (Phase C)

- Built-in AI studio panels / Avatar / TTS
- Cryptocurrency, credits, merchandise
- Hugging Face arbitrary URL execution
- Real public DHT overlay (stub + hooks only)
- Soft reputation gossip as ban authority

## License

Horde is licensed under the [PolyForm Noncommercial License 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0). See [`LICENSE`](LICENSE) for the full terms.
