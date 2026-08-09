# Wick

**Autonomous on-chain liquidation protection for Solana perpetuals.**

Wick is a Pinocchio (`no_std`) program that continuously monitors a leveraged
perps position, evaluates risk with deterministic fixed-point arithmetic, and
autonomously executes protective actions before liquidation — on venues that
allow delegated authority. See [`wick-architecture.md`](./wick-architecture.md)
for the full technical specification (health engine, action selection,
authority dispatch, delegation).

## The core problem

Existing perp protocols generally require the position **owner's signature** on
every state change, which makes truly autonomous protection architecturally
impossible there. Wick does not fake autonomy:

- **Autonomous tier** (Drift perps): the guard PDA *is* the position
  delegate. It signs a hard reduce-only `place_perp_order` itself and executes
  protective actions autonomously on breach — the sub-50ms path.
- **Co-signed tier** (Jupiter): the guard builds the owner-signed instruction
  and holds it as *pending*; the owner's signature is what lands it. The guard
  never claims to be faster than it is.

Both tiers share one guard architecture, one health engine, one action
selector, and one nonce/replay model.

## Repository structure

```
.
├── wick-architecture.md          # Technical specification + hackathon addendum
├── .github/workflows/ci.yml     # fmt + clippy(-D warnings) + build-sbf + tests
└── program/
    ├── Cargo.toml               # wick-guard — Pinocchio program (no_std, BPF)
    ├── src/
    │   ├── lib.rs               # entrypoint, module wiring
    │   ├── instruction.rs       # instruction discriminators
    │   ├── processor.rs         # handlers + §7.2 critical path (on_price_tick)
    │   ├── state.rs             # health engine, selector, partial-close solver,
    │   │                        #   dispatch regimes (§8.1–8.4)
    │   ├── account.rs           # deterministic wire-format serialization
    │   ├── delegation.rs        # MagicBlock ER delegate/commit/undelegate (§8.6)
    │   ├── drift.rs             # Drift hard reduce-only place_perp_order CPI adapter (§8.7)
    │   └── error.rs             # WickError
    ├── tests/
    │   ├── init.rs              # litesvm e2e: InitGuard → DepositMargin
    │   └── tick.rs              # litesvm e2e: autonomous + co-signed dispatch
    └── mocks/drift/             # mock Drift program for e2e CPI testing
```

## Prerequisites

- [Solana CLI](https://docs.anza.xyz/cli/) `≥ 4.0.3` (provides `cargo-build-sbf`)
- Rust toolchain `1.89`+ (CI pins `1.97.1`)
- Linux/macOS for `cargo build-sbf`

## Build

```bash
# Guard program (produces program/target/deploy/wick_guard.so)
cd program
cargo build-sbf

# Mock Drift program (needed only for the e2e tick tests)
cd program/mocks/drift
cargo build-sbf
```

## Test

```bash
cd program
cargo test --features no-entrypoint --all-targets
```

This runs 62 unit tests + 3 litesvm integration tests:

- **62 unit tests** — fixed-point health engine, breach detection, partial-close
  solver, action selection precedence + caps, 2-of-2 withdraw matrix, tick
  freshness/degraded mode, nonce semantics, serialization round-trips, Jupiter
  safety-net serialization + co-signed build persistence, the owner `Confirm`
  commit path + its rejection matrix, the verified Pyth `PriceUpdateV2`
  accessor (feed/staleness/confidence gates + 6dp scaling), and the Drift
  adapter (program ID, discriminator, reduce-only wire layout, direction
  mapping, missing-account rejection).
- **1 litesvm integration test** (`init.rs`) — `InitGuard` CPI-create + deposit.
- **2 litesvm e2e tests** (`tick.rs`):
  - *Autonomous*: an underwater Drift position on a breach tick triggers the
    guard PDA to CPI a hard reduce-only `place_perp_order` into (mock) Drift
    **signed by its own delegate seeds**, stamping the position and committing
    the nonce — the "beat the liquidator" path, proven end-to-end against a
    real SBF VM. The mock Drift enforces the delegate invariant: the CPI
    authority must be the account whose address is stored as `User.delegate`
    *and* a signer — the test fails if either is violated.
  - *Co-signed*: the same breach never reaches the venue; the action is held
    as pending and the nonce does not advance until an owner signature exists.
    On Jupiter, the guard additionally **builds** the owner-signed
    `instant_create_tpsl` safety-net instruction data and persists it beside the
    expected nonce — the owner's signature is what lands it (§8.4/§8.7). The
    owner then calls `Confirm` to record that the instruction landed on L1,
    committing the expected nonce and clearing the pending state.

Linting / formatting (what CI enforces):

```bash
cd program
cargo fmt --check
cargo clippy --features no-entrypoint --all-targets -- -D warnings
cd program/mocks/drift
cargo clippy --all-targets -- -D warnings
```

## How it works (critical path)

`OnPriceTick` runs the §7.2 ordering:

1. **Staleness** — reject ticks older than `MAX_TICK_AGE_SLOTS`; N consecutive
   stale ticks flip the guard to `degraded` (surfaced to the frontend).
2. **Health** — cross-multiplied equity-vs-maintenance check (no floats).
3. **Nonce** — monotonic tick nonce; replayed/old nonces hard-reject.
4. **Caps** — per-action + daily USD policy caps.
5. **Select** — take-profit first, then top-up, then partial-close (bounded
   binary-search solver), else escalate to manual review (never a silent no-op).
6. **Dispatch** — Autonomous: construct + CPI immediately (nonce commits only on
   a landed venue action). Co-Signed: hold the built instruction as pending
   (nonce commits only on the owner's L1 confirm).

The guard's margin wallet is a 2-of-2 (`user` + `co_authority`) — see §8.5.

## Demo narrative

1. Open a position on Drift with the guard PDA set as `User.delegate`, enroll
   it via `InitGuard` + `UpdatePosition` with the pinned market/sub-account,
   delegate the PDA to the Ephemeral Rollup.
2. Feed a breach price tick.
3. The guard evaluates health → selects a protective action → CPIs a
   reduce-only `place_perp_order` signed by its own PDA (delegate) seeds,
   exiting before liquidation.
4. Show the measured latency against the L1 baseline.
5. Explain honestly why Jupiter is co-signed (owner-signature requirement), not
   autonomous.

## Known limitations

- **Jupiter safety-net covers take-profit; defensive closes remain pending —**
  the guard builds and persists the owner-signed `instant_create_tpsl` for
  take-profit, and `Confirm` records the owner landing it. But defensive (breach)
  Jupiter closes are not yet expressed as a signed instruction — the guard holds
  those as pending state only. The full co-signed loop for breach protection
  still needs its safety-net build to close.
- **Dashboard is a static scaffold** — the frontend ships a landing page (`/`)
  and a two-column console (`/console`) with health/stats/latency/activity
  components, plus live on-chain polling for the deployed guard state, but the
  console values still fall back to demo fixtures when no guard account exists.
- **No measured latency benchmark** — the autonomous path is proven in an SBF
  VM and against the real Velocity program (p50 ≈ 375µs dispatch), but a real
  sub-50ms claim against a live L1 baseline is not yet measured on-chain.
- **Devnet program live; ER delegation not exercised** — the guard is deployed
  to devnet (`FRtyvM3xcFhL5FbukUdzaMV7t4pePiqxPvp2ZHwptBE`) and the
  delegate/commit/undelegate SDK hooks are written, but the live MagicBlock
  rollup round-trip is not yet verified.
- **Drift reduce adapter covers reduce-only orders (hard-coded)** — top-up and
  arbitrary-position orders are structurally impossible (the serializer writes
  `reduce_only=true`); a zero-size reduce escalates. Drift `market_index` +
  `subaccount_id` are pinned in guard state at init.

## Roadmap

- [x] Phase 1 — guard program, health engine, selector, solver, authority, serialization
- [x] Phase 1.5 — FlashTrade `close_position` adapter + autonomous e2e proof (retired, §8.7.1)
- [x] CI (fmt, clippy, build-sbf, tests)
- [x] Phase 3 — Jupiter co-signed safety-net adapter (build + persist the owner-signed instruction)
- [x] Phase 3.5 — owner `Confirm` instruction to land the pending Jupiter instruction + commit nonce
- [x] Phase 4 — verified Pyth `PriceUpdateV2` accessor (feed/staleness/confidence gates + 6dp scaling)
- [x] Phase 6 — Drift reduce-only `place_perp_order` adapter + delegate-PDA mock e2e (autonomous tier, §8.7)
- [x] Phase 6.5 — live-protocol proof: autonomous reduce against the real Velocity (`vELoC1...`) program in LiteSVM with real mainnet account fixtures
- [x] Phase 5 — dashboard: console + honest measured latency chart (p50 ≈ 375µs dispatch vs ~400ms L1 slot)
- [x] Frontend redesign — Ember Circuit brand; landing page (`/`) + two-column console (`/console`) on the live guard account
- [x] Deployment (partial) — guard program live on devnet (`FRtyvM3xcFhL5FbukUdzaMV7t4pePiqxPvp2ZHwptBE`); ER delegation round-trip + on-chain latency benchmark still open

## License

Apache-2.0
