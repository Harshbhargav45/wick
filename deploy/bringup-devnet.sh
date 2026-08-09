#!/usr/bin/env bash
# Devnet bring-up: extend the program account, redeploy, initialize state.
#
# Context for why each step exists:
#   - The deployed binary predates InitRouteConfig, so every init instruction
#     is rejected with 0x0 (InvalidInstruction) until the redeploy lands.
#   - `program deploy`'s implicit extend requests exactly the shortfall, and
#     the loader refuses anything under 10240 bytes, so extend runs separately.
#   - Deploy authority and cranker signer must be the same funded key, or init
#     runs as a different (empty) wallet than the one that owns the program.
set -uo pipefail

# Piping through `tee` otherwise reports tee's exit code, hiding a hard failure.
set -o pipefail

PROGRAM_ID=FRtyvM3xcFhL5FbukUdzaMV7t4pePiqxPvp2ZHwptBE
URL=devnet
REPO=/home/harsh/wick

die() { echo; echo "!! $*" >&2; exit 1; }

# ---------------------------------------------------------------- keypair ---
# `solana config get` may emit ANSI colour and trailing whitespace, either of
# which turns the captured path into a filename that cannot exist. Strip both.
KEYPAIR=$(solana config get 2>/dev/null \
  | sed 's/\x1b\[[0-9;]*[mK]//g' \
  | sed -n 's/^[[:space:]]*Keypair Path:[[:space:]]*//p' \
  | tr -d '\r' \
  | sed 's/[[:space:]]*$//')
[ -n "$KEYPAIR" ] || die "could not read keypair path from 'solana config get'"
[ -f "$KEYPAIR" ] || die "configured keypair does not exist: [$KEYPAIR]
    'solana address' works, so the CLI can read some key — this is a path
    parsing problem. Check the raw value with:
      solana config get | grep -i keypair"

strip() { sed 's/\x1b\[[0-9;]*[mK]//g' | tr -d '\r' | sed 's/[[:space:]]*$//'; }

ADDRESS=$(solana address | strip)
BALANCE=$(solana balance --url "$URL" | strip | awk '{print $1}')
echo "== keypair : $KEYPAIR"
echo "== address : $ADDRESS"
echo "== balance : $BALANCE SOL"

# Extending 20480 bytes plus deploy fees. Rent-exempt is ~0.007 SOL/KiB, so
# ~0.15 SOL covers the extend with room for the transactions themselves.
if awk "BEGIN{exit !($BALANCE < 0.35)}"; then
  die "balance $BALANCE SOL is too low for extend + deploy.
    Fund $ADDRESS with ~0.5 SOL, then re-run:
      solana airdrop 1 --url devnet"
fi

# ------------------------------------------------------------- authority ---
echo
echo "== current program state"
SHOW=$(solana program show "$PROGRAM_ID" --url "$URL" 2>&1 | sed 's/\x1b\[[0-9;]*[mK]//g')
echo "$SHOW"

if echo "$SHOW" | grep -q "Authority"; then
  AUTHORITY=$(echo "$SHOW" | sed -n 's/^[[:space:]]*Authority:[[:space:]]*//p' | sed 's/[[:space:]]*$//')
  if [ "$AUTHORITY" != "$ADDRESS" ]; then
    die "upgrade authority is $AUTHORITY but you hold $ADDRESS.
    This program cannot be upgraded by you. Deploy a fresh program instead:
      cd $REPO/program
      rm -f target/deploy/wick_guard-keypair.json
      cargo build-sbf
      solana program deploy target/deploy/wick_guard.so --url devnet
    then put the new id in cranker/.env (WICK_PROGRAM_ID) and
    frontend/.env.local (NEXT_PUBLIC_GUARD_PROGRAM_ID)."
  fi
fi

# ---------------------------------------------------------------- extend ---
echo
echo "== extend program account by 20480 bytes (loader minimum is 10240)"
solana program extend "$PROGRAM_ID" 20480 --url "$URL" \
  || echo "   (extend failed or was unnecessary — deploy will report the real reason)"

# ----------------------------------------------------------------- build ---
echo
echo "== build"
cd "$REPO/program" || die "no such directory: $REPO/program"
cargo build-sbf || die "cargo build-sbf failed"
ls -l target/deploy/wick_guard.so

# ---------------------------------------------------------------- deploy ---
echo
echo "== deploy"
solana program deploy target/deploy/wick_guard.so \
  --program-id target/deploy/wick_guard-keypair.json \
  --url "$URL" \
  || die "deploy failed — see the error above.
    If it mentions a stranded buffer, reclaim it with the printed
    'solana program close <ADDR>' command and re-run this script."

# ------------------------------------------------------------------ init ---
echo
echo "== point the cranker at the deploy keypair"
cd "$REPO/cranker" || die "no such directory: $REPO/cranker"
[ -f .env ] || cp .env.example .env
# `sed -e '$a\'` appends a newline only when the last line lacks one. Without
# it a .env with no trailing newline splices the next key onto the last value
# (DRY_RUN=1CRANKER_KEYPAIR=...), silently corrupting whatever it lands on.
grep -v '^CRANKER_KEYPAIR=' .env | sed -e '$a\' > .env.tmp
printf 'CRANKER_KEYPAIR=%s\n' "$KEYPAIR" >> .env.tmp
mv .env.tmp .env
grep -n 'CRANKER_KEYPAIR\|DRY_RUN' .env

echo
echo "== initialize on-chain state"
npm run init || die "init failed — paste the 'Program log:' lines"

# ---------------------------------------------------------------- verify ---
echo
echo "== verify accounts on chain"
node -e '
const {Connection}=require("@solana/web3.js");
import("./src/config.mjs").then(async ({config,crankerKeypair})=>{
  const {guardPda,routeConfigPda}=await import("./src/tick.mjs");
  const c=new Connection(config.rpc,"confirmed");
  const owner=crankerKeypair().publicKey;
  for (const [name,key] of [["guard",guardPda(owner).pda],["routeConfig",routeConfigPda().pda]]) {
    const i=await c.getAccountInfo(key);
    console.log(name.padEnd(12), key.toBase58(),
      i ? `len=${i.data.length} version=${i.data[0]}` : "MISSING");
  }
});'

echo
echo "== done"
