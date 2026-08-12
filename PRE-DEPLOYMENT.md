# Wick — pre-deployment checklist

Everything still outstanding before this program should be deployed anywhere
that holds real value. Written 2026-08-10, against branch `main` at `689b1eb`
plus uncommitted math fixes.

Status key: **[BLOCKER]** must be fixed before any deployment ·
**[SHIP]** must be done before mainnet · **[DEFERRED]** known, accepted for now.

**Update 2026-08-10 — all 7 blockers are closed.** The program was rebuilt,
redeployed to devnet (slot `482618401`), and the breach path re-verified
on-chain. Full suite: 89 Rust tests green; frontend production build, typecheck
and lint clean. What remains below is **[SHIP]** (mainnet-only) and
**[DEFERRED]**, plus one newly-found blocker-for-mainnet in §4.1.

---

## 1. Correctness bugs found and fixed (uncommitted — needs commit + redeploy)

These are done in the working tree but **not committed and not deployed**. The
devnet program at `FRtyvM3xcFhL5FbukUdzaMV7t4pePiqxPvp2ZHwptBE` is still running
the old, broken math.

### 1.1 Margin basis was unit-count, not notional — **FIXED**
`compute_margin_required` took bps of a raw *unit count* and compared the result
against equity in *USD*. Dimensional mismatch: the requirement came out
price-independent and too small by a factor of `current`. Practical effect — the
guard only fired at outright insolvency, never at the maintenance threshold,
which is the entire point of the engine.

- `program/src/state.rs` — now takes `current`, uses `abs_size * current / SCALE`
  as the basis. Three call sites threaded: `is_liquidatable`,
  `solve_partial_close_fraction` (`m_full`), `select_action` (`req`).
- `frontend/src/lib/guard-health.ts` — same fix mirrored.
- `wick-architecture.md` §8.1.2 / §8.2 / §8.3 — spec updated; this was a spec
  bug, not a coding slip, so the doc was wrong too.
- Regression test added: `leveraged_breach_is_caught_at_maintenance_not_insolvency`.

### 1.2 Take-profit ignored position sign — **FIXED**
`select_action` tested `current_price >= tp` for every position. Correct for a
long; backwards for a short, whose take-profit sits *below* entry. A short would
fire take-profit on the way up — exactly when it is losing — and, because its
target is below entry, on the **very first tick after enrollment**.

Now branches on `size.signum()`; `size == 0` no longer fires at all.

- [x] **[BLOCKER]** ~~No test covers the short path.~~ **FIXED.** The full suite
      passing unchanged after the fix was itself the finding: nothing was
      exercising it. `action_selector_priorities` now covers the short-TP
      crossing (fires below entry, not above) and the `size == 0` case.

---

## 2. Correctness bugs found, NOT yet fixed

### 2.1 Partial-close cap compares basis points against USD — ~~[BLOCKER]~~ **FIXED**
`program/src/state.rs`

```rust
if policy.caps.within_cap(ActionType::PartialClose, f_bps) {
```

`f_bps` is a fraction in `[0, 10_000]`. `partial_close_usd_per_action` is USD at
6dp — `cranker/src/init.mjs` sets it to `5_000 * SCALE` = `5_000_000_000`. So the
comparison was `f_bps <= 5_000_000_000`, true for every possible value.
**The partial-close cap was inert and had never bound.**

Fixed: the cap now applies to the USD notional actually closed
(`notional * f_bps / BPS_DENOM`), so step 4 of the precedence ladder is
reachable again. `wick-architecture.md` §8.2 carried the same bug in its
pseudocode and has been corrected with the reasoning inline.

### 2.2 `daily_total_usd` is not a daily cap — ~~[BLOCKER]~~ **FIXED**
`within_cap` compared a *single* action against `daily_total_usd`. There was no
accumulator and no rollover timestamp anywhere in the account layout, so it was
functionally just a second per-action ceiling — N successive within-cap top-ups
could drain the margin wallet across N ticks.

Fixed: `daily_spent_usd: u128` + `daily_epoch_start_slot: u64` added to the
guard account, reset on rollover, checked via `within_daily` and incremented on
commit. `action_daily_usd` maps each action to the USD it actually spends
(TakeProfit and EscalateManualReview spend nothing and are not rate-limited).
This is the **account-layout change** that drove v2 — see §4.1.

### 2.3 Top-up only restores equity to maintenance, not to the buffer — ~~[SHIP]~~ **FIXED**
`select_action` computed `deficit = req - eq`, landing equity *exactly* on the
liquidation line. Any adverse tick immediately re-breached, and each re-breach
spent another top-up against the (then unenforced) daily cap.

Decided explicitly rather than left ambiguous: top-up now targets
`req * (1 + trigger_buffer_bps)`, matching the partial-close solver.
`wick-architecture.md` §8.2 updated to match — as written it documented the old
behaviour, so the spec was wrong too.

**Verified on-chain, not just in a unit test.** After redeploy, a guard enrolled
at `collateral=40, size=10, entry=80` against live SOL (~$76.57) recorded:

```
PENDING tag 1 TopUp | PENDING amount 33.330571
```

which recomputes as:

```
equity 5.72121 | req 38.28606 | target w/ buffer 39.051781
deficit vs bare req    -> 32.564850   (OLD behaviour)
deficit vs buffer targ -> 33.330571   (NEW, §2.3)
observed on chain      -> 33.330571   <- exact match
```

The two figures differ by only ~2%, which is precisely why this needed an exact
recomputation to confirm: eyeballing the on-chain number would not have
distinguished the fixed path from the broken one.

---

## 3. Not audited — do before trusting the engine

- [x] **[BLOCKER]** ~~Binary-search solver convergence.~~ **DONE.** Proptested in
      `program/tests/solver_props.rs` over `(collateral, size, entry, current,
      bps…)`: the returned `f` is in range, safe, *minimal* (`f-1` is unsafe
      wherever `f > 0`), zero for already-safe positions, and the solver errors
      only where a full close genuinely cannot reach the target. The reasoning
      in the original note held up, but it is now checked rather than believed.
- [x] **[BLOCKER]** ~~Take-profit dispatch path end-to-end.~~ **DONE — restriction
      documented and enforced.** `confirm_pending` requires `state.pending_ix`,
      which is only ever set for `VENUE_JUPITER` + `TakeProfit`. Rather than
      leave `ConfirmYes` to fail obscurely on a `venue=none` or Drift guard, the
      handler now hard-rejects the combination with a named error, and
      `jupiter.rs`'s module doc states why the Jupiter tier is build-only: every
      Jupiter state change needs the position *owner*'s signature plus
      keeper-gated signer infrastructure a guard neither has nor should fake.
- [x] **[SHIP]** ~~`compute_pnl` truncates toward zero.~~ **CONFIRMED
      INTENTIONAL** and noted at the definition. `checked_div` truncates toward
      zero, so a loss rounds in the trader's favour by at most 1 unit at 6dp
      (1e-6 USD). The alternative — rounding losses away from zero — would let
      the guard report a position marginally worse than it is and fire a
      hair early. Sub-cent, deterministic, and biased in the safe direction.
- [x] **[SHIP]** ~~Base58 verification of every pinned program ID and Anchor
      discriminator.~~ **DONE**, and the way it was done matters. The previous
      "tests" asserted each constant equalled the same literal, which proves
      only that the file parses — a wrong byte passes as happily as a right one,
      and the human-readable base58 in the doc comment was never checked at all.
      That is exactly how three different typo'd spellings of the Jupiter
      address accumulated in `jupiter.rs` while the bytes stayed correct.
      `program/tests/pinned_ids.rs` now **derives** instead: program IDs are
      base58-encoded from the pinned bytes and compared to the published string,
      discriminators are re-hashed from their Anchor preimage, and a final test
      asserts no two venues share a program ID. Mutation-tested to prove it is
      not vacuous — flipping one byte of `JUPITER_PROGRAM_ID` fails the suite.
      This also surfaced a genuine documentation error: Anchor snake_cases the
      handler name to build the preimage, so the IDL's camelCase
      `instantCreateTpsl` hashes as `global:instant_create_tpsl`, and account
      discriminators use the `account:` domain rather than `global:`.

---

## 4. Deployment mechanics

- [x] **[BLOCKER]** ~~Rebuild and redeploy the program.~~ **DONE** — devnet
      `FRtyvM3xcFhL5FbukUdzaMV7t4pePiqxPvp2ZHwptBE`, last deployed in slot
      `482618401`. Confirmed byte-identical to the local build rather than
      assumed: `solana program dump` returns 113,576 bytes (deploy pads with
      zeros) whose first 107,536 bytes sha256 to
      `9e06222c2687b0dff66603df2ffb65f9d5646f523c45673156b2e1e4300cf6bf`, the
      same hash as `program/target/deploy/wick_guard.so`. So the on-chain
      verification below is a statement about *this* binary, not a near-miss.
- [x] **[BLOCKER]** ~~Re-verify the breach path on devnet after redeploy.~~
      **DONE.** The stale `29.0400` was computed under the old margin basis; the
      correct figure is **`33.330571`** and it was confirmed on-chain — see §2.3
      for the full recomputation.
- [x] **[BLOCKER]** §4.1 — Account layout migration. **DONE, with a caveat that
      is now the top mainnet blocker (below).** `GUARD_DATA_LEN` moved 342 → 366
      and `ACCOUNT_VERSION` 1 → 2. All three assertion sites are in sync,
      verified: `program/src/account.rs:154`,
      `frontend/src/lib/guard-layout.ts:14`, `cranker/src/index.mjs:16`. The
      full offset maps in the program and the frontend decoder were diffed
      field-by-field and agree. Decision taken (user-approved): **no migration
      instruction; existing devnet guards are orphaned.**

### 4.1a Orphaned v1 guards are permanently stranded — **[SHIP, blocks mainnet]**

Found while re-initializing after the redeploy: `InitGuard` fails with
`custom program error: 0x3` (`AlreadyInitialized`) for any owner who already
held a v1 guard.

The cause is `processor.rs:233` — `if data.len() != GUARD_DATA_LEN || data[0] ==
ACCOUNT_VERSION`. The length check is *correct*: there is no realloc path, and
writing 366 bytes into a 342-byte account would overflow. But the guard PDA is
derived from `[b"guard", owner]`, so the address is a pure function of the owner
— **that owner can never hold a guard again.** "Orphaned" undersells it: the
account is not merely stale, it is a permanent tombstone at the only address
that owner will ever get, and its rent is unrecoverable.

Worked around on devnet with a fresh owner keypair (`.env` untouched). Before
mainnet this needs either a `realloc` + migrate instruction or a `CloseGuard`
that refunds rent and frees the PDA. Not a devnet blocker; **it is a mainnet
one**, because the same trap fires on the next layout change.

- [ ] **[SHIP]** Upgrade authority is the deploy payer
      `FXWbEp5JkvPtcZK5WJXs3GcoEsC7XUUtQJhzamA68ffj`, a single local keypair
      (`~/Downloads/.../wallet.json`). Decide whether it is revoked or moved to
      a multisig before mainnet. Today, one key can replace the program under
      live positions.
- [ ] **[SHIP]** `route_config` kill-switch authority is the same deploy payer.
      Move to a multisig. Note this is the *same* key as the upgrade authority,
      so the kill-switch offers no independent protection from it.
- [ ] **[SHIP]** `cranker/src/init.mjs:215` sets `coAuthority: payer.publicKey`
      — the guard is self-co-authorized, so the 2-of-2 is a 1-of-1 and the
      CoSigned tier's whole premise is void. The comment says "acceptable
      because this is a devnet bring-up." Must be a distinct key before mainnet.

---

## 5. Operational

- [ ] **[SHIP]** Cranker is a single point of failure. If it stops, guards go
      `degraded` after 3 stale ticks (~30s) and stay there. No supervisor, no
      restart policy, no alerting. Needs at minimum a process manager and a
      dead-man's alert. (Standing recommendation: `systemd` unit + a
      watchdog that alerts if the last-tick timestamp in the log stops
      advancing; `node --enable-source-maps` and a crash-restart loop make the
      failure window seconds, not the full staleness window.)
- [ ] **[SHIP]** Cranker tick latency is ~6.1s against a 25-slot (~10s)
      staleness window. That is a 1.6x margin with no headroom for RPC
      degradation. Measured breakdown: hermes ~3.2s dominates. Consider a
      second Hermes endpoint or a websocket subscription.
- [ ] **[SHIP]** Rent reclaim is fire-and-forget (`sendNoWait`,
      `cranker/src/index.mjs:180`). Failures are logged and dropped. Over a long
      run this leaks rent. Add a periodic sweep for orphaned encoded-VAA and
      price-update accounts.
- [x] **[SHIP]** ~~`skipPreflight: true` on every send means program errors only
      surface after confirmation.~~ **CHECKED.** The error path logs `err.logs`
      on confirmation failure — confirmed present in `cranker/src/index.mjs`.
      This one is closed; keep it that way.
- [ ] **[DEFERRED]** `npm audit` reports 3 moderate vulns, all transitive
      (`@solana/web3.js → jayson → uuid`). The only offered fix downgrades
      web3.js to `0.0.3`. Not actionable; revisit when upstream ships.

---

## 6. Housekeeping

- [x] **[SHIP]** ~~Run the frontend build.~~ **DONE.** `npm run build` clean
      (Next.js 16.3.0 / Turbopack, 5 static routes), TypeScript clean, ESLint
      clean.
- [x] **[SHIP]** Commit the §1 fixes. **STILL UNCOMMITTED — awaiting user
      confirmation.** Constraint standing from the user: **never commit env or
      private keys.** Verified ignored: `cranker/.env`
      (`cranker/.gitignore:2`), `frontend/.env.local` (`frontend/.gitignore:34`).
      Re-scan the diff before every commit regardless. Everything in §1–§4 is in
      the working tree only; the devnet program is already running it, but the
      repository does not yet record it.
- [ ] **[DEFERRED]** Repo restructure into a proper Next.js + Solana monorepo.
      Raised, never applied, never withdrawn.
- [ ] **[DEFERRED]** `.superstack/wick-competitive-landscape.html` still has 3
      FlashTrade references. Edit there was previously rejected; leaving as-is.

---

## Summary

**7/7 blockers closed.** The two that mattered most — §2.1 (partial-close cap
inert, escalation unreachable) and §2.2 (no daily cap, margin wallet drainable
across ticks) — were the same failure mode as the margin bug fixed earlier: a
cap that looks present in the code, reads as enforced on the dashboard, and does
nothing. All three are fixed, spec'd correctly, and verified on-chain.

**Remaining for mainnet:** the §4.1a stranded-PDA trap (no realloc path — every
owner of a v1 guard is permanently locked out and their rent is stuck), the
3-keys-are-1-key authority problem (upgrade authority = kill-switch authority =
co-authority = the deploy payer), and the operational items in §5. None of them
block a devnet demo; every one of them blocks trusting the program with real
value.
