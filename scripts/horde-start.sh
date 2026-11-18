#!/usr/bin/env bash
# Start the Horde API binary (localhost operator UI at /ui/).
# Loads .env from the repo root when present. Does not print secrets.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PID_FILE="${HORDE_PID_FILE:-$ROOT/horde.pid}"
LOG_FILE="${HORDE_LOG_FILE:-$ROOT/horde.log}"
OPEN_BROWSER="${HORDE_OPEN_BROWSER:-0}"

if [[ -f "$ROOT/.env" ]]; then
  set -a
  # shellcheck disable=SC1091
  source "$ROOT/.env"
  set +a
fi

if [[ -z "${HORDE_PASSPHRASE:-}" ]]; then
  echo "HORDE_PASSPHRASE is required. Copy .env.example to .env and set it." >&2
  exit 1
fi

export HORDE_DATA_DIR="${HORDE_DATA_DIR:-./horde-data}"
export HORDE_BIND="${HORDE_BIND:-127.0.0.1:8787}"
export HORDE_UI_DIR="${HORDE_UI_DIR:-$ROOT/ui}"

if [[ -f "$PID_FILE" ]]; then
  old="$(cat "$PID_FILE" || true)"
  if [[ -n "$old" ]] && kill -0 "$old" 2>/dev/null; then
    echo "Horde already running (pid $old). Stop with scripts/horde-stop.sh" >&2
    exit 1
  fi
  rm -f "$PID_FILE"
fi

BIN=""
if [[ -x "$ROOT/target/release/horde" ]]; then
  BIN="$ROOT/target/release/horde"
elif [[ -x "$ROOT/target/debug/horde" ]]; then
  BIN="$ROOT/target/debug/horde"
else
  echo "Building horde-api (debug)…"
  cargo build -p horde-api
  BIN="$ROOT/target/debug/horde"
fi

nohup "$BIN" >"$LOG_FILE" 2>&1 &
echo $! >"$PID_FILE"
echo "Started horde (pid $(cat "$PID_FILE"))"
echo "  bind:  http://${HORDE_BIND}/"
echo "  UI:    http://${HORDE_BIND}/ui/"
echo "  log:   $LOG_FILE"
echo "Copy the studio API key from the log into the operator UI."

if [[ "$OPEN_BROWSER" == "1" || "$OPEN_BROWSER" == "true" ]]; then
  url="http://${HORDE_BIND}/ui/"
  if command -v xdg-open >/dev/null 2>&1; then
    xdg-open "$url" >/dev/null 2>&1 || true
  elif command -v open >/dev/null 2>&1; then
    open "$url" >/dev/null 2>&1 || true
  fi
fi
