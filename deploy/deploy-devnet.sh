#!/usr/bin/env bash
# Deploy the Wick guard program to devnet and print the guard PDA params.
#
# Usage:
#   ./deploy/deploy-devnet.sh              # build + deploy wick_guard.so
#   ./deploy/deploy-devnet.sh --smoke      # also 'solana program show' the result
#
# Requires: solana CLI >= 4.0.3, configured devnet keypair, cargo-build-sbf.
set -euo pipefail

SOLANA="${SOLANA:-solana}"
CLUSTER="${CLUSTER:-devnet}"
PROGRAM_DIR="$(cd "$(dirname "$0")/../program" && pwd)"
SO="$PROGRAM_DIR/target/deploy/wick_guard.so"

echo "==> Building guard .so"
(cd "$PROGRAM_DIR" && cargo build-sbf)

[[ -f "$SO" ]] || { echo "error: $SO not built" >&2; exit 1; }

echo "==> Checking cluster ($CLUSTER)"
"$SOLANA" config get >/dev/null
"$SOLANA" cluster-version --url "$CLUSTER" >/dev/null || { echo "cluster unreachable: $CLUSTER" >&2; exit 1; }

BAL=$("$SOLANA" balance --url "$CLUSTER")
echo "wallet balance: $BAL"

echo "==> Deploying program"
PROGRAM_OUT=$("$SOLANA" program deploy "$SO" --url "$CLUSTER")
echo "$PROGRAM_OUT"

PROGRAM_ID=$(echo "$PROGRAM_OUT" | grep -oP 'Program Id: \K[1-9A-HJ-NP-Za-km-z]{32,44}' | head -1 || true)
if [[ -z "${PROGRAM_ID:-}" ]]; then
  # fallback: already-deployed .so has its keypair next to it
  KP="$(dirname "$SO")/wick_guard-keypair.json"
  if [[ -f "$KP" ]]; then
    PROGRAM_ID=$("$SOLANA" address -k "$KP")
  else
    echo "could not determine program id" >&2
    exit 1
  fi
fi
echo "PROGRAM_ID=$PROGRAM_ID"

OWNER=$("$SOLANA" address --url "$CLUSTER")
echo "OWNER=$OWNER"
echo "guard PDA seeds: b\"guard\" || owner_pubkey (derive in the client SDK)"

if [[ "${1:-}" == "--smoke" ]]; then
  echo "==> program metadata"
  "$SOLANA" program show "$PROGRAM_ID" --url "$CLUSTER"
fi

echo "==> done"
echo "frontend/.env.local: NEXT_PUBLIC_GUARD_PROGRAM_ID=$PROGRAM_ID"