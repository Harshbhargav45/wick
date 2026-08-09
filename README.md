# Wick

**Autonomous on-chain liquidation protection for Solana perpetuals.**

A leveraged perp position gets liquidated because nobody was watching at the
moment it mattered. Wick is a Solana program that watches continuously, decides
with deterministic fixed-point arithmetic, and acts before the liquidator does —
on venues whose authority model actually permits it.

The guard is written in [Pinocchio](https://github.com/anza-xyz/pinocchio)
(`no_std`, no Anchor) because the critical path is a latency budget, and every
byte of deserialization on it is spend. See
[`wick-architecture.md`](./wick-architecture.md) for the full specification;
section numbers throughout this README refer to it.

## The honest version of the pitch

Most perp protocols require the position **owner's signature** on every state
change, which makes autonomous protection architecturally impossible there. Wick
does not paper over that. It runs two tiers and tells you which one you are in:

- **Autonomous** (Drift/Velocity perps) — the guard PDA *is* the position
  delegate. On breach it signs a hard reduce-only `place_perp_order` itself and
  the action lands without the owner present. This is the fast path.
- **Co-signed** (Jupiter) — the guard *builds* the owner-signed instruction and
  holds it as pending. The owner's signature is what lands it. The guard never
  claims to be faster than the human in this tier.

Both tiers share one guard account, one health engine, one action selector, and
one nonce/replay model. The difference is exactly one dispatch branch (§8.4).

## Measured latency

The guard's dispatch path, benchmarked in LiteSVM over 300 recorded dispatches
(`program/tests/latency_bench.rs`, dataset at
`frontend/public/latency-samples.json`):

| | |
|---|---|
| p50 | **187 µs** |
| p99 | 266 µs |
| min / max | 178 µs / 1396 µs |
| samples | 300 |

For scale: a Solana L1 slot is ~400 ms, and the sub-50 ms lane Wick targets is
50,000 µs — roughly **267× headroom** at p50. This is a VM-measured dispatch
cost, *not* an end-to-end on-chain claim: it excludes network propagation,
leader scheduling, and confirmation. The dashboard plots the recorded
distribution rather than a marketing number, and the target line is drawn off
scale on purpose.

## Repository structure

```
.
├── wick-architecture.md      # Technical specification
├── brand.md                  # Ember Circuit design tokens
├── .github/workflows/ci.yml  # fmt + clippy(-D warnings) + build-sbf + tests
├── program/                  # On-chain guard (Pinocchio, no_std, BPF)
│   ├── src/
│   │   ├── lib.rs            # entrypoint, module wiring
│   │   ├── instruction.rs    # instruction discriminators
│   │   ├── processor.rs      # handlers + §7.2 critical path (on_price_tick)
│   │   ├── state.rs          # health engine, selector, partial-close solver,
│   │   │                     #   dispatch regimes (§8.1–8.4)
│   │   ├── account.rs        # deterministic byte-map serialization
│   │   ├── pyth.rs           # verified PriceUpdateV2 accessor (§7.1)
│   │   ├── drift.rs          # hard reduce-only place_perp_order CPI (§8.7)
│   │   ├── jupiter.rs        # co-signed instant_create_tpsl safety net
│   │   ├── delegation.rs     # MagicBlock ER delegate/commit/undelegate (§8.6)
│   │   └── error.rs          # WickError
│   ├── tests/                # LiteSVM e2e + real-fixture proofs
│   └── mocks/drift/          # mock Drift program for e2e CPI testing
├── cranker/                  # Off-chain tick driver (Node, ESM)
│   └── src/                  # Hermes VAA fetch → post PriceUpdateV2 → OnPriceTick
├── frontend/                 # Next.js 16 console + landing page
│   └── src/
│       ├── app/              # / (landing) and /console
│       ├── components/wick/  # design-system components
│       ├── hooks/            # guard polling, derived events, wallet, actions
│       └── lib/              # account decoder, health math, instruction builders
└── deploy/deploy-devnet.sh   # build + deploy + print the guard PDA params
```

## How it works (critical path)

`OnPriceTick` runs the §7.2 ordering. The order is the design:

1. **Price** — read from the Pyth `PriceUpdateV2` account at index `[3]`, gated
   on feed ID, full verification, ≤60 s age and ≤150 bps confidence, scaled to
   6dp. It is *never* taken from the tick payload, so a cranker cannot feed the
   guard a fabricated price.
2. **Staleness** — ticks older than `MAX_TICK_AGE_SLOTS` are rejected; 3
   consecutive stale ticks flip the guard to `degraded`. A fresh tick clears
   both the streak and the flag (§8.1.3).
3. **Health** — cross-multiplied equity-vs-maintenance comparison. Fixed-point
   throughout (`SCALE = 1_000_000`, `BPS_DENOM = 10_000`): no division, no
   floats, no rounding surprises between the program and the UI.
4. **Nonce** — monotonic tick nonce; replayed or stale nonces hard-reject.
5. **Caps** — per-action and daily USD policy caps.
6. **Select** — take-profit, then top-up, then partial-close (bounded
   binary-search fraction solver in `[0, BPS_DENOM]`). If nothing inside the
   caps restores the buffer it escalates to manual review — never a silent
   no-op.
7. **Dispatch** — *Autonomous*: build and CPI immediately; the nonce commits on
   the landed venue action. *Co-signed*: persist the built instruction as
   pending; the nonce commits only when the owner confirms on L1.

The guard's margin wallet is 2-of-2 (`owner` + `co_authority`) — see §8.5. A
singleton `RouteConfig` PDA holds a kill-switch checked at the top of every
state-mutating instruction.

## Instruction surface

| # | Instruction | Signer | Purpose |
|---|---|---|---|
| 0 | `InitGuard` | owner + payer | Create the guard PDA (`b"guard" \|\| owner`) and pin its policy |
| 1 | `DepositMargin` | owner | Add collateral |
| 2 | `WithdrawMargin` | owner **+** co-authority | 2-of-2 withdrawal (§8.5) |
| 3 | `SetPaused` | route authority | Kill-switch |
| 4 | `Delegate` | owner | Delegate the guard to the Ephemeral Rollup (§8.6) |
| 5 | `CommitAndUndelegate` | owner | Commit state to L1 and exit the rollup |
| 6 | `Commit` | owner | Commit state, stay delegated |
| 7 | `OnPriceTick` | cranker | The §7.2 critical path |
| 8 | `UpdatePosition` | owner | Enroll/refresh the watched position snapshot |
| 9 | `ConfirmYes` | owner | Record that the co-signed instruction landed; commit the nonce |
| 10 | `InitRouteConfig` | route authority | Create the singleton config |

## Running it

### Prerequisites

- [Solana CLI](https://docs.anza.xyz/cli/) `≥ 4.0.3` (provides `cargo-build-sbf`)
- Rust `1.89`+ (CI pins `1.97.1`), Linux or macOS
- Node `20+` for the cranker and frontend

### Build and test the program

```bash
cd program
cargo build-sbf                                    # -> target/deploy/wick_guard.so
cargo test --features no-entrypoint --all-targets

cd mocks/drift && cargo build-sbf                  # only for the e2e tick tests
```

What CI enforces:

```bash
cargo fmt --check
cargo clippy --features no-entrypoint --all-targets -- -D warnings
```

### Deploy to devnet

```bash
./deploy/deploy-devnet.sh            # add --smoke to print program metadata
```

It prints the `PROGRAM_ID` to put in `frontend/.env.local`.

### Run the console

```bash
cd frontend
cp .env.example .env.local           # set NEXT_PUBLIC_GUARD_PROGRAM_ID
npm install && npm run dev
```

`/` is the landing page; `/console` attaches to a live guard. With a wallet
connected it resolves *your* guard by PDA and enables the co-sign confirm; with
no wallet it stays read-only. It renders explicit unconfigured / no-guard /
error states rather than falling back to fake numbers, and the activity feed
records only transitions it actually observes — the guard account holds current
state, not history, so nothing is backfilled.

### Run the cranker

```bash
cd cranker
cp .env.example .env                 # RPC, keypair path, guard + feed addresses
npm install && npm start
```

It pulls a VAA from Hermes, posts a fully-verified `PriceUpdateV2` through the
Pyth receiver, and drives `OnPriceTick`.

> Secrets stay out of git: `.env`, `*.pem`, `*.key`, and any `*keypair*.json`
> are gitignored. Only `.env.example` templates are tracked.

## Testing

61 unit tests plus LiteSVM integration tests covering:

- the fixed-point health engine, breach detection, and the partial-close solver
- action-selection precedence and cap enforcement
- the 2-of-2 withdraw matrix and nonce semantics
- serialization round-trips for every account layout
- the verified Pyth accessor — feed, staleness, confidence, 6dp scaling
- the Drift adapter — program ID, discriminator, reduce-only wire layout,
  direction mapping, missing-account rejection

The end-to-end proofs are the interesting ones:

- **Autonomous** (`tick.rs`) — an underwater Drift position on a breach tick
  drives the guard PDA to CPI a hard reduce-only `place_perp_order` into mock
  Drift **signed by its own delegate seeds**, stamping the position and
  committing the nonce. The mock enforces the delegate invariant: the CPI
  authority must be the account stored as `User.delegate` *and* a signer — the
  test fails if either is violated.
- **Co-signed** (`tick.rs`) — the same breach never reaches the venue. The
  action is held pending, the nonce does not advance, and on Jupiter the guard
  additionally builds and persists the owner-signed `instant_create_tpsl` data
  beside the expected nonce. `Confirm` then commits it.
- **Real protocol** (`real_drift.rs`) — the autonomous reduce runs against the
  real Velocity (`vELoC1…`) program in LiteSVM using mainnet account fixtures,
  not a mock.

## Security notes

- The guard PDA is a **delegate**, never an owner. Delegates cannot withdraw.
- `reduce_only = true` is written by the serializer itself (`d[30] = 1`), so a
  position-increasing order is not merely unused — it is unconstructible.
- Re-initializing a funded guard account is refused, closing the path where an
  attacker passes a victim's guard with their own key as `owner` to reset nonce
  and collateral.
- Account layouts are explicit byte maps with pinned offsets rather than
  `repr(C)` casts, so BPF and the TypeScript decoder cannot disagree.
- `frontend/src/lib/guard-layout.ts` and `guard-health.ts` mirror
  `account.rs` and `state.rs` byte for byte and in bigint respectively — the UI
  never disagrees with the program about whether a position is liquidatable.

## Known limitations

Stated plainly, because a risk tool that oversells itself is worse than none:

- **Jupiter defensive closes are not yet built as signed instructions.** The
  take-profit safety net is built, persisted, and confirmable. Breach closes on
  Jupiter are held as pending state only; that build path is still open.
- **The sub-50 ms claim is VM-measured, not on-chain.** 187 µs p50 is real and
  reproducible, but it is dispatch cost in LiteSVM — it excludes propagation and
  confirmation.
- **ER delegation is written but not round-tripped.** The guard is live on
  devnet (`FRtyvM3xcFhL5FbukUdzaMV7t4pePiqxPvp2ZHwptBE`) and the
  delegate/commit/undelegate hooks exist, but the live MagicBlock round trip is
  unverified.
- **The console is read + confirm.** It resolves the guard, decodes it, and can
  land `ConfirmYes`. Init, deposit, and position enrollment are built as
  instruction builders but not yet surfaced as forms.
- **Drift market and sub-account are pinned at init** and cannot be changed
  without re-initialization.
- `npm audit` reports 3 moderate advisories, all transitive through
  `@solana/web3.js → jayson → uuid`. The only offered fix downgrades web3.js to
  `0.0.3`, so it is deliberately not applied.

## Roadmap

- [x] Guard program — health engine, selector, solver, authority dispatch, serialization
- [x] CI — fmt, clippy, build-sbf, tests
- [x] Jupiter co-signed safety net + owner `Confirm`
- [x] Verified Pyth `PriceUpdateV2` accessor
- [x] Drift reduce-only adapter + delegate-PDA e2e (autonomous tier)
- [x] Live-protocol proof against real Velocity with mainnet fixtures
- [x] Measured latency benchmark (187 µs p50) + honest dashboard chart
- [x] Devnet deployment + cranker driving real price ticks
- [x] Frontend — Ember Circuit brand, landing page, live console, wallet co-sign
- [ ] Jupiter defensive-close instruction build
- [ ] ER delegation round-trip on live MagicBlock
- [ ] On-chain end-to-end latency measurement

## License

Apache-2.0
