/**
 * The v3 guard wire layout, in one place.
 *
 * These offsets mirror `program/src/account.rs`. They were previously copied
 * into five modules, which is how `GUARD_DATA_LEN = 366` and
 * `ACCOUNT_VERSION = 2` survived the v3 bump in three of them: a stale length
 * makes `getProgramAccounts`' `dataSize` filter match nothing, so the cranker
 * reports "no guard accounts found" against a chain where the guard is present
 * and breaching. Nothing throws — the loop just protects nothing, forever.
 *
 * One module, imported everywhere, so the next layout change is one edit and
 * `test/guard-layout.test.mjs` pins the numbers against the Rust source.
 */
import { PublicKey } from "@solana/web3.js";

/** Total serialized length of a guard account (v3). */
export const GUARD_DATA_LEN = 416;

/**
 * Version badge in byte 0. One value for all three account types — the guard,
 * the singleton route config, and the margin reserve all stamp and check the
 * same constant, so a version bump invalidates all of them together.
 */
export const ACCOUNT_VERSION = 3;

/** `RouteConfig` — [version, authority(32), paused]. */
export const ROUTE_CONFIG_LEN = 34;

/** `WalletState` — [version, owner(32), co_authority(32), balance u128]. */
export const WALLET_DATA_LEN = 81;

/** Byte offsets into the guard account. */
export const G = {
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
  pendingIxData: 287,
  driftMarket: 338,
  driftSubaccount: 340,
  dailySpentUsd: 342,
  dailyEpochStartTs: 358,
  // --- v3 additions ---
  venueSize: 366,
  venueCollateral: 382,
  reconcileTs: 398,
  reconcileNonce: 406,
  reconcileStatus: 414,
  marginWalletBump: 415,
};

export const SCALE = 1_000_000n;
export const BPS_DENOM = 10_000n;
export const U128_MAX = (1n << 128n) - 1n;

export const VENUE_NONE = 0;
export const VENUE_JUPITER = 2;
export const VENUE_DRIFT = 3;
export const VENUE_NAMES = { 0: "none", 2: "jupiter", 3: "drift" };

/** `reconcile_status` values, mirroring `account.rs`. */
export const RECONCILE_NEVER = 0;
export const RECONCILE_CONVERGED = 1;
export const RECONCILE_DIVERGED = 2;
export const RECONCILE_NAMES = {
  0: "never",
  1: "converged",
  2: "DIVERGED",
};

/** `pending_ix.kind` values (§8.7). Both legs are `instant_create_tpsl`, so the
 * tag is the only thing distinguishing a take-profit from a stop. */
export const PENDING_IX_NONE = 0;
export const PENDING_IX_JUPITER_TPSL = 1;
export const PENDING_IX_JUPITER_DEFENSIVE_CLOSE = 2;
export const PENDING_IX_NAMES = {
  0: "none",
  1: "jupiter take-profit",
  2: "jupiter defensive close",
};
export const PENDING_IX_DATA_LEN = 50;

/**
 * Instruction discriminators, mirroring the `WickInstruction` enum in
 * `program/src/instruction.rs` and the dispatch in `processor.rs`.
 *
 * Keep every variant here even where the cranker never sends it. A partial map
 * is how the previous version of this object came to claim `UpdatePosition: 3`
 * and `SetPaused: 12`: the gaps left by the delegation instructions got closed
 * up, silently renumbering everything after them.
 */
export const IX = {
  InitGuard: 0,
  DepositMargin: 1,
  WithdrawMargin: 2,
  SetPaused: 3,
  Delegate: 4,
  CommitAndUndelegate: 5,
  Commit: 6,
  OnPriceTick: 7,
  UpdatePosition: 8,
  ConfirmYes: 9,
  InitRouteConfig: 10,
  CloseGuard: 11,
  SetRouteAuthority: 12,
  ReconcileVenue: 13,
  InitMarginWallet: 14,
  FundMarginWallet: 15,
  WithdrawMarginWallet: 16,
};

export function readU128LE(d, off) {
  let v = 0n;
  for (let i = 15; i >= 0; i--) v = (v << 8n) | BigInt(d[off + i]);
  return v;
}

export function readI128LE(d, off) {
  const v = readU128LE(d, off);
  return v >= 1n << 127n ? v - (1n << 128n) : v;
}

export function writeU128LE(buf, value, off) {
  let v = BigInt(value);
  for (let i = 0; i < 16; i++) {
    buf[off + i] = Number(v & 0xffn);
    v >>= 8n;
  }
  return buf;
}

/**
 * True when `data` is a live, current-version guard body.
 *
 * A delegated or closed guard is all zeroes (the delegation program zeroes the
 * PDA before assigning it away), which passes a length check and a naive
 * version check on byte 0 only because that byte is also zero — hence both.
 */
export function isGuardAccount(data) {
  return data?.length === GUARD_DATA_LEN && data[G.version] === ACCOUNT_VERSION;
}

/**
 * Decode the fields the cranker actually acts on. Deliberately partial: the
 * inspector prints from raw offsets so it still says something useful about an
 * account this decoder would reject.
 */
export function decodeGuard(data) {
  if (!isGuardAccount(data)) {
    throw new Error(
      `not a v${ACCOUNT_VERSION} guard: len=${data?.length} version=${data?.[G.version]}`
    );
  }
  const pendingIxKind = data[G.pendingIxKind];
  return {
    version: data[G.version],
    venue: data[G.venue],
    venueOwner: new PublicKey(data.subarray(G.venueOwner, G.venueOwner + 32)),
    coAuthority: new PublicKey(data.subarray(G.coAuthority, G.coAuthority + 32)),
    authorityReq: data[G.authorityReq],
    takeProfit: readU128LE(data, G.takeProfit),
    collateral: readU128LE(data, G.collateral),
    size: readI128LE(data, G.size),
    entry: readU128LE(data, G.entry),
    price: readU128LE(data, G.price),
    nonce: data.readBigUInt64LE(G.nonce),
    lastCheckTs: data.readBigInt64LE(G.lastCheckTs),
    pendingTag: data[G.pendingTag],
    degraded: data[G.degraded] !== 0,
    staleStreak: data[G.staleStreak],
    dailySpentUsd: readU128LE(data, G.dailySpentUsd),
    pendingIx:
      pendingIxKind === PENDING_IX_NONE
        ? null
        : {
            kind: pendingIxKind,
            expectedNonce: data.readBigUInt64LE(G.pendingIxNonce),
            data: Buffer.from(
              data.subarray(G.pendingIxData, G.pendingIxData + PENDING_IX_DATA_LEN)
            ),
          },
    driftMarketIndex: data.readUInt16LE(G.driftMarket),
    driftSubaccountId: data.readUInt16LE(G.driftSubaccount),
    venueSize: readI128LE(data, G.venueSize),
    venueCollateral: readU128LE(data, G.venueCollateral),
    reconcileTs: data.readBigInt64LE(G.reconcileTs),
    reconcileNonce: data.readBigUInt64LE(G.reconcileNonce),
    reconcileStatus: data[G.reconcileStatus],
    marginWalletBump: data[G.marginWalletBump],
  };
}
