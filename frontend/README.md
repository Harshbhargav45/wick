# Wick console

The Next.js frontend for Wick: the landing page at `/` and the live guard
console at `/console`.

## Setup

```bash
cp .env.example .env.local   # set NEXT_PUBLIC_GUARD_PROGRAM_ID
npm install
npm run dev
```

`NEXT_PUBLIC_SOLANA_CLUSTER` drives Explorer links and should match the RPC
cluster. RPC endpoints are tried in order (Helius → QuickNode → explicit →
public devnet), and `NEXT_PUBLIC_*` values are inlined into the browser bundle,
so treat any key in this file as public.

## Console behavior

- With a wallet connected it resolves *your* guard PDA (`b"guard" || owner`).
  Without one it scans program accounts and stays read-only.
- A wallet with no guard gets a create flow: initialize RouteConfig if the
  program has none, then create the guard and enroll a position.
- Writes quote the same blocking reason everywhere (no wallet, missing/paused
  RouteConfig, wrong owner). 2-of-2 withdrawals produce a partially signed
  transaction instead of pretending to send one key short.
- Addresses, reserve, and transaction signatures link to Solana Explorer.

The guard account holds current state, not history, so the activity feed records
only transitions observed during the session and never backfills.

## Scripts

| Command | Purpose |
|---|---|
| `npm run dev` | Dev server |
| `npm run build` | Production build |
| `npm run start` | Serve the production build |
| `npm run lint` | ESLint |

The pure logic in `src/lib` has its own narrow build and test runner:

```bash
npx tsc -p tsconfig.test.json
node --test test/
```

That compiles `amounts`, `guard-layout`, `guard-health`, `guard-init`,
`guard-program`, and `explorer` to CommonJS in `.test-build/`; no browser runner
or extra dependency. `.test-build/` is gitignored and ESLint-ignored.

## Layout

```
src/app/            routes: / (landing), /console, layout + theme boot script
src/components/wick design-system and console panels
src/hooks/          guard polling, derived events, wallet, write actions, motion
src/lib/            account decoders, health math, payload builders, RPC/Explorer
test/               node --test specs against .test-build
```

`src/lib/guard-layout.ts` mirrors `program/src/account.rs` and
`src/lib/guard-health.ts` mirrors `program/src/state.rs`; they must move
together with the program.
