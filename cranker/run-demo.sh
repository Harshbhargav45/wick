#!/usr/bin/env bash
# Demo supervisor: the cranker exits on any error that escapes the tick
# try/catch (RPC ETIMEDOUT, websocket 429). Losing the process mid-recording
# degrades the guard after 3 stale ticks, so restart it instead.
cd "$(dirname "$0")" || exit 1
export DRY_RUN=0
export TICK_INTERVAL_MS="${TICK_INTERVAL_MS:-3000}"
while true; do
  echo "[supervisor] starting cranker $(date +%H:%M:%S)"
  node src/index.mjs
  echo "[supervisor] cranker exited ($?) — restarting in 2s"
  sleep 2
done
