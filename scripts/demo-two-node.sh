#!/usr/bin/env bash
# Horde Phase B — two-node loopback mesh demo.
# Boots inviter (A) + joiner (B), mints/redeems a mesh invite, runs a
# cross-node stub job with routing: mesh, prints the attested result.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/horde-target}"
export RUST_LOG="${RUST_LOG:-info}"

API_A="${HORDE_DEMO_API_A:-127.0.0.1:8787}"
API_B="${HORDE_DEMO_API_B:-127.0.0.1:8788}"
MESH_A="${HORDE_DEMO_MESH_A:-127.0.0.1:9797}"
MESH_B="${HORDE_DEMO_MESH_B:-127.0.0.1:9798}"

RUN_DIR="${HORDE_DEMO_DIR:-$(mktemp -d /tmp/horde-demo-XXXXXX)}"
DATA_A="$RUN_DIR/data-a"
DATA_B="$RUN_DIR/data-b"
LOG_A="$RUN_DIR/node-a.log"
LOG_B="$RUN_DIR/node-b.log"
PID_A=""
PID_B=""
PASS=0

cleanup() {
  local code=$?
  if [[ -n "$PID_A" ]] && kill -0 "$PID_A" 2>/dev/null; then
    kill "$PID_A" 2>/dev/null || true
    wait "$PID_A" 2>/dev/null || true
  fi
  if [[ -n "$PID_B" ]] && kill -0 "$PID_B" 2>/dev/null; then
    kill "$PID_B" 2>/dev/null || true
    wait "$PID_B" 2>/dev/null || true
  fi
  # Remove demo data unless HORDE_DEMO_KEEP=1
  if [[ "${HORDE_DEMO_KEEP:-0}" != "1" ]]; then
    rm -rf "$RUN_DIR"
  else
    echo "kept demo dir: $RUN_DIR" >&2
  fi
  if [[ $PASS -eq 1 ]]; then
    echo
    echo "=== DEMO PASSED ==="
  elif [[ $code -ne 0 ]]; then
    echo
    echo "=== DEMO FAILED (exit $code) ===" >&2
    if [[ -f "$LOG_A" ]]; then
      echo "--- node-a log (tail) ---" >&2
      tail -n 40 "$LOG_A" >&2 || true
    fi
    if [[ -f "$LOG_B" ]]; then
      echo "--- node-b log (tail) ---" >&2
      tail -n 40 "$LOG_B" >&2 || true
    fi
  fi
}
trap cleanup EXIT

die() {
  echo "error: $*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

need cargo
need curl
need jq

echo "==> building horde binary"
cargo build -p horde-api --quiet
BIN="$CARGO_TARGET_DIR/debug/horde"
[[ -x "$BIN" ]] || die "binary not found at $BIN"

mkdir -p "$DATA_A" "$DATA_B"

# Ephemeral keystore passphrases (override via env; never commit real secrets).
rand_pass() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 16
  else
    # Fallback: /dev/urandom hex
    head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n'
  fi
}
PASS_A="${HORDE_PASSPHRASE_A:-$(rand_pass)}"
PASS_B="${HORDE_PASSPHRASE_B:-$(rand_pass)}"

echo "==> starting node A (inviter/worker) on $API_A mesh $MESH_A"
HORDE_PASSPHRASE="$PASS_A" \
HORDE_DATA_DIR="$DATA_A" \
HORDE_HOSTNAME=node-a \
HORDE_BIND="$API_A" \
HORDE_MESH_BIND="$MESH_A" \
"$BIN" >"$LOG_A" 2>&1 &
PID_A=$!

echo "==> starting node B (joiner/requester) on $API_B mesh $MESH_B"
HORDE_PASSPHRASE="$PASS_B" \
HORDE_DATA_DIR="$DATA_B" \
HORDE_HOSTNAME=node-b \
HORDE_BIND="$API_B" \
HORDE_MESH_BIND="$MESH_B" \
HORDE_BOOTSTRAP_PEERS="$MESH_A" \
"$BIN" >"$LOG_B" 2>&1 &
PID_B=$!

wait_health() {
  local url=$1 name=$2
  local i
  for i in $(seq 1 60); do
    if curl -sf "http://$url/health" >/dev/null 2>&1; then
      echo "    $name healthy"
      return 0
    fi
    if [[ "$name" == "A" && -n "$PID_A" ]] && ! kill -0 "$PID_A" 2>/dev/null; then
      die "node A exited early — see $LOG_A"
    fi
    if [[ "$name" == "B" && -n "$PID_B" ]] && ! kill -0 "$PID_B" 2>/dev/null; then
      die "node B exited early — see $LOG_B"
    fi
    sleep 0.25
  done
  die "timeout waiting for $name health at http://$url/health"
}

echo "==> waiting for health"
wait_health "$API_A" A
wait_health "$API_B" B

extract_api_key() {
  local log=$1
  local key
  key="$(grep -oE 'hk_dev_[A-Za-z0-9]+' "$log" | head -n1 || true)"
  [[ -n "$key" ]] || die "could not parse API key from $log"
  echo "$key"
}

KEY_A="$(extract_api_key "$LOG_A")"
KEY_B="$(extract_api_key "$LOG_B")"
echo "==> API keys acquired"

auth_a() { curl -sf -H "Authorization: Bearer $KEY_A" -H 'Content-Type: application/json' "$@"; }
auth_b() { curl -sf -H "Authorization: Bearer $KEY_B" -H 'Content-Type: application/json' "$@"; }

echo "==> minting mesh_join invite on A"
INVITE_JSON="$(auth_a -X POST "http://$API_A/v1/admin/invites" \
  -d '{"purpose":"mesh_join","ttl_secs":600,"note":"demo-node-b"}')"
TOKEN="$(echo "$INVITE_JSON" | jq -r '.token')"
[[ -n "$TOKEN" && "$TOKEN" != "null" ]] || die "mint invite failed: $INVITE_JSON"
echo "    token_id=$(echo "$INVITE_JSON" | jq -r '.token_id')"

echo "==> B joining A's mesh ($MESH_A)"
JOIN_JSON="$(auth_b -X POST "http://$API_B/v1/admin/mesh/join" \
  -d "$(jq -nc --arg addr "$MESH_A" --arg token "$TOKEN" '{peer_addr:$addr,token:$token}')")"
PEER_ID="$(echo "$JOIN_JSON" | jq -r '.node_id')"
[[ -n "$PEER_ID" && "$PEER_ID" != "null" ]] || die "mesh join failed: $JOIN_JSON"
echo "    peer node_id=$PEER_ID"

echo "==> submitting cross-node job from B (routing: mesh)"
JOB_JSON="$(auth_b -X POST "http://$API_B/v1/jobs" \
  -d '{
    "fingerprint": {
      "model_id": "stub/echo",
      "revision_hash": "deadbeefcafebabe",
      "precision": "fp16"
    },
    "kind": "stub_echo",
    "payload": "hello from B via mesh",
    "routing": "mesh"
  }')"
JOB_ID="$(echo "$JOB_JSON" | jq -r '.id')"
[[ -n "$JOB_ID" && "$JOB_ID" != "null" ]] || die "job submit failed: $JOB_JSON"
echo "    job_id=$JOB_ID status=$(echo "$JOB_JSON" | jq -r '.status')"

echo "==> waiting for job completion"
STATUS=""
ERROR=""
for _ in $(seq 1 80); do
  STATUS_JSON="$(auth_b "http://$API_B/v1/jobs/$JOB_ID")"
  STATUS="$(echo "$STATUS_JSON" | jq -r '.status')"
  ERROR="$(echo "$STATUS_JSON" | jq -r '.error // empty')"
  case "$STATUS" in
    succeeded|failed|cancelled)
      break
      ;;
  esac
  sleep 0.25
done

echo "    final status=$STATUS"

if [[ "$STATUS" != "succeeded" ]]; then
  echo "job did not succeed: status=$STATUS error=${ERROR:-none}" >&2
  echo "$STATUS_JSON" | jq . >&2 || true
  exit 1
fi

echo "==> fetching attested result"
RESULT_JSON="$(auth_b "http://$API_B/v1/jobs/$JOB_ID/result")"
OUTPUT="$(echo "$RESULT_JSON" | jq -r '.output')"
# worker_node_id is a transparent hex pubkey (NodeId); show a short form.
WORKER_HEX="$(echo "$RESULT_JSON" | jq -r '.attestation.worker_node_id')"
WORKER="${WORKER_HEX:0:32}…"
RUNTIME="$(echo "$RESULT_JSON" | jq -r '.attestation.runtime')"
SIG="$(echo "$RESULT_JSON" | jq -r '.attestation.signature_hex')"
FP="$(echo "$RESULT_JSON" | jq -r '.attestation.model_fingerprint.model_id')"

echo
echo "--- attested result ---"
echo "output:          $OUTPUT"
echo "worker_node_id:  $WORKER"
echo "model_id:        $FP"
echo "runtime:         $RUNTIME"
echo "signature_hex:   ${SIG:0:32}…"
echo "-----------------------"

echo "==> requester accept"
ACCEPT_JSON="$(auth_b -X POST "http://$API_B/v1/jobs/$JOB_ID/accept")"
DECISION="$(echo "$ACCEPT_JSON" | jq -r '.result_decision')"
echo "    result_decision=$DECISION"

[[ "$OUTPUT" == *"hello from B via mesh"* ]] || die "unexpected output (missing payload echo)"
[[ "$DECISION" == "accepted" ]] || die "accept did not stick: $DECISION"

PASS=1
