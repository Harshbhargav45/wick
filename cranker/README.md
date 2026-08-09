# Wick cranker — devnet driver for OnPriceTick

Fetches live SOL/USD price updates from Pyth and posts them to the guard
program, driving the `OnPriceTick` path so the console has real data.

## Setup

```bash
cd cranker
npm install
cp .env.example .env   # then fill in your wallet path + RPC
```

## Run

```bash
npm run post-vaa       # one-shot: post latest VAA to Pyth receiver, dry-run build tick
npm start              # loop: every INTERVAL_SECS, fetch + post + tick
```

## What it does

1. Asks Hermes for the latest SOL/USD `PriceUpdateV2` (base64 VAA).
2. Posts the VAA to the Pyth Solana receiver on the Solana cluster using
   the receiver IDL directly (avoids the bundled `PythSolanaReceiver`
   class, whose `jito-ts` dependency chain is broken on Node >= 18).
3. Derives the guard + route_config PDAs, builds the `OnPriceTick`
   instruction (`[0] guard, [1] clock, [2] route_config, [3] PriceUpdateV2`),
   and sends it with a fresh monotonic nonce.

The program requires the `PriceUpdateV2` to be created via the **full
posting path** (Wormhole VAA verification) so it carries
`verification_level == Full`; `postUpdateAtomic` is partial verification and
is rejected by the guard.

## Env

See `.env.example` for all variables.