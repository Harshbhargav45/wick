import { strict as assert } from 'node:assert';
import { describe, it } from 'node:test';

import {
  INIT_GUARD_DATA_LEN,
  INIT_GUARD_POLICY_LEN,
  IX_INIT_GUARD,
  IX_INIT_ROUTE_CONFIG,
  U128_MAX,
  buildInitGuardData,
  buildInitRouteConfigData,
  writeI128LE,
  writeU128LE,
} from '../.test-build/guard-init.js';

/*
 * Offsets restated from `cranker/src/init.mjs` / `parse_policy`, not imported,
 * so a drift in the serializer fails here instead of initializing a guard with
 * a corrupted policy.
 */
const P = {
  venue: 0,
  coAuthority: 1,
  authorityReq: 33,
  maintenanceBps: 34,
  triggerBufferBps: 50,
  feeBps: 66,
  capTopUp: 82,
  capPartialClose: 98,
  capDaily: 114,
  takeProfit: 130,
  driftMarketIndex: 146,
  driftSubaccountId: 148,
};

function u128(data, off) {
  let acc = 0n;
  for (let i = 15; i >= 0; i--) acc = (acc << 8n) | BigInt(data[off + i]);
  return acc;
}

const CO_AUTHORITY = Uint8Array.from({ length: 32 }, (_, i) => i + 1);

function policy(overrides = {}) {
  return {
    venue: 3,
    coAuthority: CO_AUTHORITY,
    authorityReq: 0,
    maintenanceBps: 500n,
    triggerBufferBps: 200n,
    feeBps: 10n,
    topUpUsdPerAction: 1_000_000_000n,
    partialCloseUsdPerAction: 5_000_000_000n,
    dailyTotalUsd: 20_000_000_000n,
    takeProfit: null,
    driftMarketIndex: 1,
    driftSubaccountId: 2,
    ...overrides,
  };
}

describe('init route config payload', () => {
  it('is the discriminator and bump only', () => {
    assert.deepEqual([...buildInitRouteConfigData(255)], [IX_INIT_ROUTE_CONFIG, 255]);
    assert.equal(IX_INIT_ROUTE_CONFIG, 10);
  });
});

describe('init guard payload', () => {
  it('has the layout parse_policy reads', () => {
    const data = buildInitGuardData(254, policy());
    const p = 2;

    assert.equal(data.length, INIT_GUARD_DATA_LEN);
    assert.equal(data.length, 2 + INIT_GUARD_POLICY_LEN);
    assert.equal(data[0], IX_INIT_GUARD);
    assert.equal(data[1], 254);
    assert.equal(data[p + P.venue], 3);
    assert.deepEqual(data.slice(p + P.coAuthority, p + P.coAuthority + 32), CO_AUTHORITY);
    assert.equal(data[p + P.authorityReq], 0);
    assert.equal(u128(data, p + P.maintenanceBps), 500n);
    assert.equal(u128(data, p + P.triggerBufferBps), 200n);
    assert.equal(u128(data, p + P.feeBps), 10n);
    assert.equal(u128(data, p + P.capTopUp), 1_000_000_000n);
    assert.equal(u128(data, p + P.capPartialClose), 5_000_000_000n);
    assert.equal(u128(data, p + P.capDaily), 20_000_000_000n);
    assert.equal(data[p + P.driftMarketIndex], 1);
    assert.equal(data[p + P.driftMarketIndex + 1], 0);
    assert.equal(data[p + P.driftSubaccountId], 2);
  });

  it('writes the no-take-profit sentinel when takeProfit is null', () => {
    const data = buildInitGuardData(255, policy({ takeProfit: null }));
    assert.equal(u128(data, 2 + P.takeProfit), U128_MAX);
  });

  it('writes a real take-profit instead of the sentinel', () => {
    const data = buildInitGuardData(255, policy({ takeProfit: 123_456_789n }));
    assert.equal(u128(data, 2 + P.takeProfit), 123_456_789n);
  });

  it('carries the co-signed authority byte', () => {
    const data = buildInitGuardData(255, policy({ authorityReq: 1 }));
    assert.equal(data[2 + P.authorityReq], 1);
  });

  it('rejects a co-authority that is not 32 bytes', () => {
    assert.throws(() => buildInitGuardData(255, policy({ coAuthority: new Uint8Array(31) })), /32 bytes/);
  });

  it('refuses values that do not fit their wire width', () => {
    assert.throws(() => writeU128LE(new Uint8Array(16), 0, U128_MAX + 1n), /u128/);
    assert.throws(() => writeU128LE(new Uint8Array(16), 0, -1n), /u128/);
    assert.throws(() => buildInitGuardData(255, policy({ driftMarketIndex: 0x10000 })), /u16/);
    assert.throws(() => buildInitGuardData(255, policy({ driftSubaccountId: -1 })), /u16/);
  });
});

describe('signed i128 encoding', () => {
  it('round-trips a negative size as two\'s complement', () => {
    const out = new Uint8Array(16);
    writeI128LE(out, 0, -2_500_000n);
    assert.equal(u128(out, 0), (1n << 128n) - 2_500_000n);
  });

  it('rejects values outside i128', () => {
    assert.throws(() => writeI128LE(new Uint8Array(16), 0, 1n << 127n), /i128/);
    assert.throws(() => writeI128LE(new Uint8Array(16), 0, -(1n << 127n) - 1n), /i128/);
  });
});
