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

- **Autonomous tier** (FlashTrade): the guard PDA *is* the position owner. Once
  the guard PDA is delegated to a MagicBlock Ephemeral Rollup, it signs and
  executes protective actions itself — this is the sub-50ms path.
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
    │   ├── flash.rs             # FlashTrade close_position CPI adapter (§8.7)
    │   └── error.rs             # WickError
    ├── tests/
    │   ├── init.rs              # litesvm e2e: InitGuard → DepositMargin
    │   └── tick.rs              # litesvm e2e: autonomous + co-signed dispatch
    └── mocks/flash/             # mock Flash program for e2e CPI testing
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

# Mock Flash program (needed only for the e2e tick tests)
cd program/mocks/flash
cargo build-sbf
```

## Test

```bash
cd program
cargo test --features no-entrypoint --all-targets
```

This runs 48 unit tests + 3 litesvm integration tests:

- **48 unit tests** — fixed-point health engine, breach detection, partial-close
  solver, action selection precedence + caps, 2-of-2 withdraw matrix, tick
  freshness/degraded mode, nonce semantics, serialization round-trips, Jupiter
  safety-net serialization + co-signed build persistence.
- **1 litesvm integration test** (`init.rs`) — `InitGuard` CPI-create + deposit.
- **2 litesvm e2e tests** (`tick.rs`):
  - *Autonomous*: an underwater position on a breach tick triggers the guard
    PDA to CPI `close_position` into (mock) Flash **signed by its own seeds**,
    stamping the position and committing the nonce — the "beat the liquidator"
    path, proven end-to-end against a real SBF VM.
  - *Co-signed*: the same breach never reaches the venue; the action is held
    as pending and the nonce does not advance until an owner signature exists.
    On Jupiter, the guard additionally **builds** the owner-signed
    `instant_create_tpsl` safety-net instruction data and persists it beside the
    expected nonce — the owner's signature is what lands it (§8.4/§8.7).

Linting / formatting (what CI enforces):

```bash
cd program
cargo fmt --check
cargo clippy --features no-entrypoint --all-targets -- -D warnings
cd program/mocks/flash
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

1. Open a position on FlashTrade with the guard PDA as owner, enroll it via
   `InitGuard` + `UpdatePosition`, delegate the PDA to the Ephemeral Rollup.
2. Feed a breach price tick.
3. The guard evaluates health → selects a protective action → CPIs
   `close_position` signed by its own PDA seeds, exiting before liquidation.
4. Show the measured latency against the L1 baseline.
5. Explain honestly why Jupiter is co-signed (owner-signature requirement), not
   autonomous.

## Known limitations

- **Jupiter adapter covers the safety-net build only** — the guard builds and
  persists the owner-signed `instant_create_tpsl` instruction for take-profit,
  but there is no `Confirm` instruction to land it, and defensive (breach)
  Jupiter closes are not yet expressed as a signed instruction — the guard holds
  those as pending state only. The co-signed path needs the owner's L1
  signature to fully close the loop.
- **No frontend/dashboard yet** — guard state is readable on-chain but there is
  no UI or latency chart.
- **No measured latency benchmark** — the autonomous path is proven in an SBF
  VM, but a real sub-50ms claim against a live L1 baseline is not yet measured.
- **No deployed devnet program** — the ER delegate/commit/undelegate round-trip
  is implemented as SDK hooks but not exercised against a live MagicBlock
  rollup.
- **Flash adapter covers `close_position` only** — top-up and true partial-close
  (fractional) CPI variants are not implemented; a partial-close/take-profit
  currently resolves to a protective full close.
- **Mock venue in tests** — the e2e CPI runs against a mock Flash program that
  models the owner-signer invariant, not the real protocol deployment.

## Roadmap

- [x] Phase 1 — guard program, health engine, selector, solver, authority, serialization
- [x] Phase 1.5 — FlashTrade `close_position` adapter + autonomous e2e proof
- [x] CI (fmt, clippy, build-sbf, tests)
- [x] Phase 3 — Jupiter co-signed safety-net adapter (build + persist the owner-signed instruction)
- [ ] Phase 3.5 — owner `Confirm` instruction to land the pending Jupiter instruction + commit nonce
- [ ] Phase 4 — Pyth Lazer price wiring (if Flash V2's own stream is insufficient)
- [ ] Phase 5 — dashboard (real latency chart first, then state panels)
- [ ] Deployment — devnet program, live ER delegation round-trip, latency benchmark

## License

Apache-2.0
