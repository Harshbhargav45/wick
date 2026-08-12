import { strict as assert } from 'node:assert';
import { describe, it } from 'node:test';

import {
  ACCOUNT_VERSION,
  GUARD_DATA_LEN,
  PENDING_IX_JUPITER_DEFENSIVE_CLOSE,
  PENDING_IX_JUPITER_TPSL,
  RECONCILE_CONVERGED,
  RECONCILE_DIVERGED,
  WALLET_DATA_LEN,
  decodeGuardState,
  decodeWalletState,
} from '../.test-build/guard-layout.js';
import { computeHealth, dailyBudget, DAILY_EPOCH_SECS } from '../.test-build/guard-health.js';

/*
 * Byte offsets, restated here on purpose.
 *
 * The decoder is only correct relative to `program/src/account.rs`, so a test
 * that imported the decoder's own constants would agree with it no matter what
 * it did. These are transcribed from the Rust struct instead, which is what
 * makes a silent renumbering fail here.
 */
const OFF = {
  version: 0,
  venue: 1,
  venueOwner: 2,
  coAuthority: 34,
  authorityReq: 66,
  maintenanceBps: 67,
  triggerBufferBps: 83,
  feeBps: 99,
  capTopUp: 115,
  capPartialClose: 131,
  capDaily: 147,
  takeProfit: 163,
  collateral: 179,
  size: 195,
  entry: 211,
  price: 227,
  nonce: 243,
  lastCheckTs: 251,
  pendingTag: 259,
  pendingAmount: 260,
  degraded: 276,
  staleStreak: 277,
  pendingIxKind: 278,
  pendingIxNonce: 279,
  driftMarket: 338,
  driftSubaccount: 340,
  dailySpentUsd: 342,
  dailyEpochStartTs: 358,
  venueSize: 366,
  venueCollateral: 382,
  reconcileTs: 398,
  reconcileNonce: 406,
  reconcileStatus: 414,
  marginWalletBump: 415,
};

const SCALE = 1_000_000n;

function putU128(buf, off, value) {
  let v = value < 0n ? (1n << 128n) + value : value;
  for (let i = 0; i < 16; i++) {
    buf[off + i] = Number(v & 0xffn);
    v >>= 8n;
  }
}

function putU64(buf, off, value) {
  let v = value < 0n ? (1n << 64n) + value : value;
  for (let i = 0; i < 8; i++) {
    buf[off + i] = Number(v & 0xffn);
    v >>= 8n;
  }
}

function putU16(buf, off, value) {
  buf[off] = value & 0xff;
  buf[off + 1] = (value >> 8) & 0xff;
}

/** A well-formed v3 guard: 10 units long at $100, $1000 collateral, healthy. */
function buildGuard(overrides = {}) {
  const buf = new Uint8Array(GUARD_DATA_LEN);
  buf[OFF.version] = ACCOUNT_VERSION;
  buf[OFF.venue] = 3; // Drift
  buf.fill(7, OFF.venueOwner, OFF.coAuthority);
  buf.fill(9, OFF.coAuthority, OFF.authorityReq);
  buf[OFF.authorityReq] = 0; // Autonomous
  putU128(buf, OFF.maintenanceBps, 500n);
  putU128(buf, OFF.triggerBufferBps, 1_500n);
  putU128(buf, OFF.feeBps, 10n);
  putU128(buf, OFF.capTopUp, 500n * SCALE);
  putU128(buf, OFF.capPartialClose, 2_000n * SCALE);
  putU128(buf, OFF.capDaily, 1_000n * SCALE);
  putU128(buf, OFF.takeProfit, (1n << 128n) - 1n); // sentinel: none
  putU128(buf, OFF.collateral, 1_000n * SCALE);
  putU128(buf, OFF.size, 10n * SCALE);
  putU128(buf, OFF.entry, 100n * SCALE);
  putU128(buf, OFF.price, 100n * SCALE);
  putU64(buf, OFF.nonce, 42n);
  putU64(buf, OFF.lastCheckTs, 1_700_000_000n);
  putU16(buf, OFF.driftMarket, 1);
  putU16(buf, OFF.driftSubaccount, 2);
  putU128(buf, OFF.dailySpentUsd, 100n * SCALE);
  putU64(buf, OFF.dailyEpochStartTs, 1_700_000_000n);
  buf[OFF.reconcileStatus] = RECONCILE_CONVERGED;
  putU64(buf, OFF.reconcileTs, 1_700_000_000n);
  putU64(buf, OFF.reconcileNonce, 42n);
  putU128(buf, OFF.venueSize, 10n * SCALE);
  putU128(buf, OFF.venueCollateral, 1_000n * SCALE);
  buf[OFF.marginWalletBump] = 254;

  for (const [key, value] of Object.entries(overrides)) {
    if (typeof value === 'number') buf[OFF[key]] = value;
    else if (key.endsWith('Ts') || key === 'nonce' || key === 'reconcileNonce')
      putU64(buf, OFF[key], value);
    else putU128(buf, OFF[key], value);
  }
  return buf;
}

describe('guard state decoding', () => {
  it('reads a v3 guard at the offsets the program writes', () => {
    const s = decodeGuardState(buildGuard());
    assert.equal(s.venue, 3);
    assert.equal(s.authorityReq, 'Autonomous');
    assert.equal(s.policy.maintenanceBps, 500n);
    assert.equal(s.policy.triggerBufferBps, 1_500n);
    assert.equal(s.policy.caps.topUpUsdPerAction, 500n * SCALE);
    assert.equal(s.policy.takeProfit, null, 'u128::MAX is the "unset" sentinel, not a price');
    assert.equal(s.collateral, 1_000n * SCALE);
    assert.equal(s.size, 10n * SCALE);
    assert.equal(s.nonce, 42n);
    assert.equal(s.lastCheckTs, 1_700_000_000n);
    assert.equal(s.driftMarketIndex, 1);
    assert.equal(s.driftSubaccountId, 2);
    assert.equal(s.marginWalletBump, 254);
    assert.equal(s.venueOwner.length, 32);
    assert.equal(s.coAuthority.length, 32);
  });

  it('rejects a v2-length account rather than misreading it', () => {
    // The whole reason this decoder was rewritten: it declared 366 and threw on
    // every real account. A wrong length must fail loudly, either direction.
    assert.throws(() => decodeGuardState(new Uint8Array(366)), /expected 416 bytes, got 366/);
  });

  it('rejects an unsupported version', () => {
    const buf = buildGuard();
    buf[OFF.version] = 2;
    assert.throws(() => decodeGuardState(buf), /unsupported account version 2/);
  });

  it('treats last_check_ts and daily_epoch_start_ts as signed', () => {
    // The program stores `i64`. Read unsigned, a negative timestamp becomes
    // ~1.8e19 and every staleness and epoch comparison silently inverts.
    const buf = buildGuard();
    putU64(buf, OFF.lastCheckTs, -5n);
    putU64(buf, OFF.dailyEpochStartTs, -86_400n);
    const s = decodeGuardState(buf);
    assert.equal(s.lastCheckTs, -5n);
    assert.equal(s.dailyEpochStartTs, -86_400n);
  });

  it('decodes a short position as a negative size', () => {
    const buf = buildGuard();
    putU128(buf, OFF.size, -10n * SCALE);
    assert.equal(decodeGuardState(buf).size, -10n * SCALE);
  });

  it('reads a bump of 0 as "no reserve linked"', () => {
    const buf = buildGuard();
    buf[OFF.marginWalletBump] = 0;
    assert.equal(decodeGuardState(buf).marginWalletBump, null);
  });

  it('accepts a staged Jupiter defensive close', () => {
    // Tag 2 did not exist before §8.9 and the old decoder threw on it, so any
    // guard holding one failed to render at all.
    for (const kind of [PENDING_IX_JUPITER_TPSL, PENDING_IX_JUPITER_DEFENSIVE_CLOSE]) {
      const buf = buildGuard();
      buf[OFF.pendingIxKind] = kind;
      putU64(buf, OFF.pendingIxNonce, 43n);
      const s = decodeGuardState(buf);
      assert.equal(s.pendingIxKind, kind);
      assert.equal(s.pendingIxNonce, 43n);
    }
  });

  it('reports no pending nonce when nothing is staged', () => {
    assert.equal(decodeGuardState(buildGuard()).pendingIxNonce, null);
  });

  it('rejects tags the program cannot have written', () => {
    const badIx = buildGuard();
    badIx[OFF.pendingIxKind] = 3;
    assert.throws(() => decodeGuardState(badIx), /unknown pending_ix kind 3/);

    const badStatus = buildGuard();
    badStatus[OFF.reconcileStatus] = 3;
    assert.throws(() => decodeGuardState(badStatus), /unknown reconcile status 3/);

    const badPending = buildGuard();
    badPending[OFF.pendingTag] = 9;
    assert.throws(() => decodeGuardState(badPending), /unknown pending tag 9/);
  });

  it('decodes each pending action tag', () => {
    const cases = [
      [1, { kind: 'TopUp', amount: 250n * SCALE }],
      [2, { kind: 'PartialClose', fractionBps: 2_500n }],
      [3, { kind: 'TakeProfit' }],
      [4, { kind: 'EscalateManualReview' }],
    ];
    for (const [tag, expected] of cases) {
      const buf = buildGuard();
      buf[OFF.pendingTag] = tag;
      putU128(buf, OFF.pendingAmount, expected.amount ?? expected.fractionBps ?? 0n);
      assert.deepEqual(decodeGuardState(buf).pending, expected);
    }
  });

  it('carries the venue snapshot and verdict', () => {
    const buf = buildGuard();
    buf[OFF.reconcileStatus] = RECONCILE_DIVERGED;
    putU128(buf, OFF.venueSize, 4n * SCALE);
    putU128(buf, OFF.venueCollateral, 900n * SCALE);
    const { reconcile } = decodeGuardState(buf);
    assert.equal(reconcile.status, RECONCILE_DIVERGED);
    assert.equal(reconcile.venueSize, 4n * SCALE);
    assert.equal(reconcile.venueCollateral, 900n * SCALE);
    assert.equal(reconcile.ts, 1_700_000_000n);
  });
});

describe('wallet state decoding', () => {
  function buildWallet(balance = 2_000_000_000n) {
    const buf = new Uint8Array(WALLET_DATA_LEN);
    buf[0] = ACCOUNT_VERSION;
    buf.fill(3, 1, 33);
    buf.fill(4, 33, 65);
    putU128(buf, 65, balance);
    return buf;
  }

  it('reads owner, co-authority and balance', () => {
    const w = decodeWalletState(buildWallet());
    assert.equal(w.owner.length, 32);
    assert.equal(w.coAuthority.length, 32);
    assert.equal(w.balance, 2_000_000_000n);
  });

  it('rejects a wrong length or version', () => {
    assert.throws(() => decodeWalletState(new Uint8Array(80)), /expected 81 bytes/);
    const bad = buildWallet();
    bad[0] = 2;
    assert.throws(() => decodeWalletState(bad), /unsupported account version/);
  });
});

describe('health', () => {
  it('takes maintenance margin on notional, not on the unit count', () => {
    // 10 units at $100 = $1000 notional; 5% = $50. Taking bps of the raw unit
    // count would give $0.005 and make the requirement price-independent.
    const h = computeHealth(decodeGuardState(buildGuard()));
    assert.equal(h.notional, 1_000n * SCALE);
    assert.equal(h.marginRequired, 50n * SCALE);
    assert.equal(h.triggerTarget, 57n * SCALE + 500_000n); // 50 * 1.15
    assert.equal(h.pnl, 0n);
    assert.equal(h.equity, 1_000n * SCALE);
    assert.equal(h.liquidatable, false);
    assert.equal(h.breachingBuffer, false);
    assert.equal(h.unprotected, false);
  });

  it('flags liquidatable below maintenance', () => {
    const buf = buildGuard();
    putU128(buf, OFF.collateral, 40n * SCALE);
    const h = computeHealth(decodeGuardState(buf));
    assert.equal(h.liquidatable, true);
    assert.equal(h.breachingBuffer, false, 'liquidatable supersedes the buffer breach');
    assert.equal(h.unprotected, true);
  });

  it('flags a buffer breach above maintenance', () => {
    const buf = buildGuard();
    putU128(buf, OFF.collateral, 55n * SCALE);
    const h = computeHealth(decodeGuardState(buf));
    assert.equal(h.liquidatable, false);
    assert.equal(h.breachingBuffer, true);
  });

  it('carries losses into equity for a short', () => {
    const buf = buildGuard();
    putU128(buf, OFF.size, -10n * SCALE);
    putU128(buf, OFF.price, 110n * SCALE); // price up, short loses
    const h = computeHealth(decodeGuardState(buf));
    assert.equal(h.pnl, -100n * SCALE);
    assert.equal(h.equity, 900n * SCALE);
  });

  it('treats a diverged guard as unprotected even while the math reads healthy', () => {
    const buf = buildGuard();
    buf[OFF.reconcileStatus] = RECONCILE_DIVERGED;
    const h = computeHealth(decodeGuardState(buf));
    assert.equal(h.diverged, true);
    assert.equal(h.liquidatable, false, 'equity is fine — that is the point');
    assert.equal(h.breachingBuffer, false);
    assert.equal(
      h.unprotected,
      true,
      'the program refuses to execute, so the console must not report this as protected',
    );
  });

  it('treats a degraded guard as unprotected', () => {
    const buf = buildGuard();
    buf[OFF.degraded] = 1;
    assert.equal(computeHealth(decodeGuardState(buf)).unprotected, true);
  });
});

describe('daily budget', () => {
  const EPOCH = 1_700_000_000n;

  it('counts spend inside the current epoch', () => {
    const b = dailyBudget(decodeGuardState(buildGuard()), EPOCH + 100n);
    assert.equal(b.spent, 100n * SCALE);
    assert.equal(b.total, 1_000n * SCALE);
    assert.equal(b.remaining, 900n * SCALE);
    assert.equal(b.exhausted, false);
  });

  it('rolls over after DAILY_EPOCH_SECS, not after 216000 slots', () => {
    // The bug this replaced: 216_000 was a slot count. Counted as seconds it is
    // 2.5 days, so a rolled-over epoch still reported as spent for 1.5 days.
    const state = decodeGuardState(buildGuard());
    assert.equal(DAILY_EPOCH_SECS, 86_400n);
    assert.equal(dailyBudget(state, EPOCH + 86_399n).spent, 100n * SCALE);
    assert.equal(dailyBudget(state, EPOCH + 86_400n).spent, 0n, 'exactly at the boundary');
    assert.equal(dailyBudget(state, EPOCH + 216_000n).spent, 0n);
  });

  it('does not go negative when the chain clock is behind the epoch start', () => {
    const b = dailyBudget(decodeGuardState(buildGuard()), EPOCH - 500n);
    assert.equal(b.spent, 100n * SCALE);
    assert.equal(b.remaining, 900n * SCALE);
  });

  it('reports an exhausted budget when spend meets the cap', () => {
    const buf = buildGuard();
    putU128(buf, OFF.dailySpentUsd, 1_000n * SCALE);
    const b = dailyBudget(decodeGuardState(buf), EPOCH + 1n);
    assert.equal(b.remaining, 0n);
    assert.equal(b.exhausted, true);
  });
});
