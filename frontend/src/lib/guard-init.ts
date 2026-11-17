/**
 * Instruction payloads for the two initialization handlers, mirrored from
 * `cranker/src/init.mjs` and `program/src/processor.rs`.
 *
 * Deliberately free of `@solana/web3.js`: everything here is byte serialization,
 * so the Node test build can load it without the browser-oriented Solana stack.
 * The address assembly stays in `guard-program.ts`.
 */

export const IX_INIT_GUARD = 0;
export const IX_INIT_ROUTE_CONFIG = 10;

/** `u128::MAX` — the program's "no take-profit set" sentinel. */
export const U128_MAX = (1n << 128n) - 1n;

const I128_MIN = -(1n << 127n);
const I128_MAX = (1n << 127n) - 1n;

/** `[disc, bump]` — the whole payload of `InitRouteConfig`. */
export const INIT_ROUTE_CONFIG_DATA_LEN = 2;

/** Policy blob after the discriminator and bump, as `parse_policy` reads it. */
export const INIT_GUARD_POLICY_LEN = 150;
export const INIT_GUARD_DATA_LEN = 2 + INIT_GUARD_POLICY_LEN;

/** Policy field offsets — relative to the start of the blob, not the data. */
export const POLICY_OFF = {
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
} as const;

export interface InitGuardPolicy {
  venue: number;
  /** 32 raw bytes; the handler stores them verbatim. */
  coAuthority: Uint8Array;
  /** 0 = autonomous (guard signs), 1 = co-signed (owner signs). */
  authorityReq: 0 | 1;
  maintenanceBps: bigint;
  triggerBufferBps: bigint;
  feeBps: bigint;
  topUpUsdPerAction: bigint;
  partialCloseUsdPerAction: bigint;
  dailyTotalUsd: bigint;
  /** `null` serializes as `U128_MAX` (no take-profit). */
  takeProfit: bigint | null;
  driftMarketIndex: number;
  driftSubaccountId: number;
}

export function writeU128LE(out: Uint8Array, off: number, value: bigint): void {
  if (value < 0n || value > U128_MAX) throw new Error('value does not fit in u128');
  let v = value;
  for (let i = 0; i < 16; i++) {
    out[off + i] = Number(v & 0xffn);
    v >>= 8n;
  }
}

export function writeI128LE(out: Uint8Array, off: number, value: bigint): void {
  if (value < I128_MIN || value > I128_MAX) throw new Error('value does not fit in i128');
  writeU128LE(out, off, value < 0n ? (1n << 128n) + value : value);
}

export function writeU16LE(out: Uint8Array, off: number, value: number): void {
  if (!Number.isInteger(value) || value < 0 || value > 0xffff) {
    throw new Error('value does not fit in u16');
  }
  out[off] = value & 0xff;
  out[off + 1] = (value >> 8) & 0xff;
}

/**
 * `[disc=0, bump, policy(150)]`.
 *
 * Offsets here are the ones `cranker/src/init.mjs` writes, which are in turn the
 * ones `parse_policy` reads. A drift of one byte does not fail cleanly — it
 * initializes a guard with a corrupted policy — so the tests restate them.
 */
export function buildInitGuardData(bump: number, policy: InitGuardPolicy): Uint8Array {
  if (policy.coAuthority.length !== 32) {
    throw new Error('co-authority must be 32 bytes');
  }
  const data = new Uint8Array(INIT_GUARD_DATA_LEN);
  data[0] = IX_INIT_GUARD;
  data[1] = bump;

  const p = 2;
  data[p + POLICY_OFF.venue] = policy.venue;
  data.set(policy.coAuthority, p + POLICY_OFF.coAuthority);
  data[p + POLICY_OFF.authorityReq] = policy.authorityReq;

  writeU128LE(data, p + POLICY_OFF.maintenanceBps, policy.maintenanceBps);
  writeU128LE(data, p + POLICY_OFF.triggerBufferBps, policy.triggerBufferBps);
  writeU128LE(data, p + POLICY_OFF.feeBps, policy.feeBps);
  writeU128LE(data, p + POLICY_OFF.capTopUp, policy.topUpUsdPerAction);
  writeU128LE(data, p + POLICY_OFF.capPartialClose, policy.partialCloseUsdPerAction);
  writeU128LE(data, p + POLICY_OFF.capDaily, policy.dailyTotalUsd);
  writeU128LE(
    data,
    p + POLICY_OFF.takeProfit,
    policy.takeProfit === null ? U128_MAX : policy.takeProfit,
  );

  writeU16LE(data, p + POLICY_OFF.driftMarketIndex, policy.driftMarketIndex);
  writeU16LE(data, p + POLICY_OFF.driftSubaccountId, policy.driftSubaccountId);
  return data;
}

/** `[disc=10, bump]` — the account itself is created by the handler's CPI. */
export function buildInitRouteConfigData(bump: number): Uint8Array {
  const data = new Uint8Array(INIT_ROUTE_CONFIG_DATA_LEN);
  data[0] = IX_INIT_ROUTE_CONFIG;
  data[1] = bump;
  return data;
}
