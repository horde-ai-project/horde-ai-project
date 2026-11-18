#!/usr/bin/env bash
# Stop a Horde process started by horde-start.sh.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PID_FILE="${HORDE_PID_FILE:-$ROOT/horde.pid}"

if [[ ! -f "$PID_FILE" ]]; then
  echo "No pid file at $PID_FILE"
  exit 0
fi

pid="$(cat "$PID_FILE")"
if [[ -z "$pid" ]]; then
  rm -f "$PID_FILE"
  echo "Empty pid file removed."
  exit 0
fi

if kill -0 "$pid" 2>/dev/null; then
  kill "$pid" || true
  # Wait briefly for shutdown
  for _ in 1 2 3 4 5; do
    if ! kill -0 "$pid" 2>/dev/null; then
      break
    fi
    sleep 0.2
  done
  if kill -0 "$pid" 2>/dev/null; then
    kill -9 "$pid" || true
  fi
  echo "Stopped horde (pid $pid)"
else
  echo "Process $pid not running"
fi
rm -f "$PID_FILE"
