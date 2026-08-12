/**
 * The cranker's guard layout is a hand-copied mirror of
 * `program/src/account.rs`. That copy already went stale once: the program went
 * to v3 (416 bytes) while three cranker modules still declared 366/v2, which
 * makes the `dataSize` filter match nothing — the loop reports "no guard
 * accounts found" against a chain where the guard exists and is breaching.
 * Nothing throws, so nothing catches it.
 *
 * These tests read the Rust source and assert the numbers agree. They fail the
 * moment the program's layout moves and the mirror does not.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import {
  ACCOUNT_VERSION,
  G,
  GUARD_DATA_LEN,
  PENDING_IX_DATA_LEN,
  PENDING_IX_JUPITER_DEFENSIVE_CLOSE,
  RECONCILE_DIVERGED,
  decodeGuard,
  isGuardAccount,
  readI128LE,
  readU128LE,
  writeU128LE,
} from "../src/guard-layout.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const accountRs = readFileSync(join(here, "../../program/src/account.rs"), "utf8");

/** Pull `const NAME: usize = 123;` (or `u8`/`pub const`) out of the Rust source. */
function rustConst(name) {
  const m = accountRs.match(
    new RegExp(`(?:pub )?const ${name}:\\s*(?:usize|u8|u32)\\s*=\\s*(\\d+)`)
  );
  assert.ok(m, `${name} not found in account.rs — was it renamed?`);
  return Number(m[1]);
}

test("length and version agree with the program", () => {
  assert.equal(GUARD_DATA_LEN, rustConst("GUARD_DATA_LEN"));
  assert.equal(ACCOUNT_VERSION, rustConst("ACCOUNT_VERSION"));
  assert.equal(PENDING_IX_DATA_LEN, rustConst("PENDING_IX_DATA_LEN"));
});

test("every mirrored offset matches account.rs", () => {
  const pairs = [
    ["venue", "G_VENUE_OFF"],
    ["venueOwner", "G_VENUE_OWNER_OFF"],
    ["coAuthority", "G_CO_AUTH_OFF"],
    ["authorityReq", "G_AUTH_REQ_OFF"],
    ["maintenanceBps", "G_MAINT_OFF"],
    ["triggerBufferBps", "G_BUF_OFF"],
    ["feeBps", "G_FEE_OFF"],
    ["capTopUp", "G_CAP_TOP_OFF"],
    ["capPartialClose", "G_CAP_PARTIAL_OFF"],
    ["capDaily", "G_CAP_DAILY_OFF"],
    ["takeProfit", "G_TP_OFF"],
    ["collateral", "G_COLLAT_OFF"],
    ["size", "G_SIZE_OFF"],
    ["entry", "G_ENTRY_OFF"],
    ["price", "G_PRICE_OFF"],
    ["nonce", "G_NONCE_OFF"],
    ["lastCheckTs", "G_SLOT_OFF"],
    ["pendingTag", "G_PENDING_TAG_OFF"],
    ["pendingAmount", "G_PENDING_AMT_OFF"],
    ["degraded", "G_DEGRADED_OFF"],
    ["staleStreak", "G_STALE_STREAK_OFF"],
    ["pendingIxKind", "G_PX_TAG_OFF"],
    ["pendingIxNonce", "G_PX_NONCE_OFF"],
    ["pendingIxData", "G_PX_DATA_OFF"],
    ["driftMarket", "G_DRIFT_MARKET_OFF"],
    ["driftSubaccount", "G_DRIFT_SUBACCOUNT_OFF"],
    ["dailySpentUsd", "G_DAILY_SPENT_OFF"],
    ["dailyEpochStartTs", "G_DAILY_EPOCH_OFF"],
    ["venueSize", "G_RECON_VENUE_SIZE_OFF"],
    ["venueCollateral", "G_RECON_VENUE_COLLAT_OFF"],
    ["reconcileTs", "G_RECON_TS_OFF"],
    ["reconcileNonce", "G_RECON_NONCE_OFF"],
    ["reconcileStatus", "G_RECON_STATUS_OFF"],
    ["marginWalletBump", "G_MARGIN_WALLET_BUMP_OFF"],
  ];
  for (const [js, rs] of pairs) {
    assert.equal(G[js], rustConst(rs), `G.${js} disagrees with ${rs}`);
  }
});

test("reconcile and pending-ix tags agree with the program", () => {
  assert.equal(RECONCILE_DIVERGED, rustConst("RECONCILE_DIVERGED"));
  assert.equal(
    PENDING_IX_JUPITER_DEFENSIVE_CLOSE,
    rustConst("PENDING_IX_JUPITER_DEFENSIVE_CLOSE")
  );
});

test("the v3 tail fits inside the declared length", () => {
  assert.equal(G.marginWalletBump + 1, GUARD_DATA_LEN);
  assert.ok(G.pendingIxData + PENDING_IX_DATA_LEN <= G.driftMarket);
});

test("u128 round-trips little-endian, signed and unsigned", () => {
  const buf = Buffer.alloc(16);
  writeU128LE(buf, 1_234_567_890n, 0);
  assert.equal(readU128LE(buf, 0), 1_234_567_890n);
  assert.equal(buf[0], 0xd2, "not little-endian");

  // -1 as i128 is all-ones; reading it unsigned must not be mistaken for a
  // real balance, which is why size uses the signed reader.
  const neg = Buffer.alloc(16, 0xff);
  assert.equal(readI128LE(neg, 0), -1n);
  assert.equal(readU128LE(neg, 0), (1n << 128n) - 1n);
});

/** A minimal but structurally valid guard body. */
function guardFixture(over = {}) {
  const d = Buffer.alloc(GUARD_DATA_LEN);
  d[G.version] = ACCOUNT_VERSION;
  d[G.venue] = over.venue ?? 3;
  d.fill(7, G.venueOwner, G.venueOwner + 32);
  d.fill(8, G.coAuthority, G.coAuthority + 32);
  writeU128LE(d, over.collateral ?? 100_000_000n, G.collateral);
  writeU128LE(d, over.size ?? 100_000_000n, G.size);
  writeU128LE(d, 50_000_000n, G.entry);
  writeU128LE(d, 50_000_000n, G.price);
  d.writeBigUInt64LE(over.nonce ?? 4n, G.nonce);
  d.writeBigInt64LE(1_700_000_000n, G.lastCheckTs);
  d.writeUInt16LE(over.market ?? 0, G.driftMarket);
  d.writeUInt16LE(over.subaccount ?? 0, G.driftSubaccount);
  d.writeBigUInt64LE(over.reconcileNonce ?? 0n, G.reconcileNonce);
  d[G.reconcileStatus] = over.reconcileStatus ?? 0;
  d[G.marginWalletBump] = over.marginWalletBump ?? 0;
  if (over.pendingIxKind) {
    d[G.pendingIxKind] = over.pendingIxKind;
    d.writeBigUInt64LE(over.pendingIxNonce ?? 5n, G.pendingIxNonce);
    d.fill(0xab, G.pendingIxData, G.pendingIxData + PENDING_IX_DATA_LEN);
  }
  return d;
}

test("decodeGuard reads the v3 fields the cranker acts on", () => {
  const g = decodeGuard(
    guardFixture({
      market: 3,
      subaccount: 7,
      reconcileNonce: 11n,
      reconcileStatus: RECONCILE_DIVERGED,
      marginWalletBump: 254,
      pendingIxKind: PENDING_IX_JUPITER_DEFENSIVE_CLOSE,
    })
  );
  assert.equal(g.version, ACCOUNT_VERSION);
  assert.equal(g.nonce, 4n);
  assert.equal(g.size, 100_000_000n);
  assert.equal(g.driftMarketIndex, 3);
  assert.equal(g.driftSubaccountId, 7);
  assert.equal(g.reconcileNonce, 11n);
  assert.equal(g.reconcileStatus, RECONCILE_DIVERGED);
  assert.equal(g.marginWalletBump, 254);
  assert.equal(g.pendingIx.kind, PENDING_IX_JUPITER_DEFENSIVE_CLOSE);
  assert.equal(g.pendingIx.expectedNonce, 5n);
  assert.equal(g.pendingIx.data.length, PENDING_IX_DATA_LEN);
});

test("a short position decodes negative rather than enormous", () => {
  const d = guardFixture();
  writeU128LE(d, (1n << 128n) - 100_000_000n, G.size); // -100 as i128
  assert.equal(decodeGuard(d).size, -100_000_000n);
});

test("no pending ix decodes as null, not a zero-filled one", () => {
  assert.equal(decodeGuard(guardFixture()).pendingIx, null);
});

test("a stale-version or wrong-length account is rejected, not misread", () => {
  const v2 = guardFixture();
  v2[G.version] = 2;
  assert.equal(isGuardAccount(v2), false);
  assert.throws(() => decodeGuard(v2), /not a v3 guard/);

  const short = Buffer.alloc(366, 3);
  assert.equal(isGuardAccount(short), false);
  assert.throws(() => decodeGuard(short), /len=366/);
});

test("a delegated or closed guard is all zero and rejected", () => {
  // delegate_account zeroes the PDA before assigning it away. Byte 0 is then
  // 0, so a version check alone would have to be against the right value —
  // this pins that the zero body does not read as a live guard.
  assert.equal(isGuardAccount(Buffer.alloc(GUARD_DATA_LEN)), false);
});
