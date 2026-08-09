/**
 * Wire-format decoder for the `PositionGuard` account.
 *
 * This mirrors `GuardState::from_bytes` in `program/src/account.rs` byte for
 * byte. The offsets below are the same constants the program uses; if the
 * on-chain layout changes, both sides have to move together.
 */

export const ACCOUNT_VERSION = 1;
export const GUARD_DATA_LEN = 342;
export const PENDING_IX_DATA_LEN = 50;

export const SCALE = 1_000_000n;
export const BPS_DENOM = 10_000n;

/** No venue adapter: the guard watches and records but never dispatches. */
export const VENUE_NONE = 0;
export const VENUE_JUPITER = 2;
export const VENUE_DRIFT = 3;

const G_VENUE_OFF = 1;
const G_VENUE_OWNER_OFF = 2;
const G_CO_AUTH_OFF = 34;
const G_AUTH_REQ_OFF = 66;
const G_MAINT_OFF = 67;
const G_BUF_OFF = 83;
const G_FEE_OFF = 99;
const G_CAP_TOP_OFF = 115;
const G_CAP_PARTIAL_OFF = 131;
const G_CAP_DAILY_OFF = 147;
const G_TP_OFF = 163;
const G_COLLAT_OFF = 179;
const G_SIZE_OFF = 195;
const G_ENTRY_OFF = 211;
const G_PRICE_OFF = 227;
const G_NONCE_OFF = 243;
const G_SLOT_OFF = 251;
const G_PENDING_TAG_OFF = 259;
const G_PENDING_AMT_OFF = 260;
const G_DEGRADED_OFF = 276;
const G_STALE_STREAK_OFF = 277;
const G_PX_TAG_OFF = 278;
const G_PX_NONCE_OFF = 279;
const G_DRIFT_MARKET_OFF = 338;
const G_DRIFT_SUBACCOUNT_OFF = 340;

/** `u128::MAX` — the sentinel the program uses for "no take-profit set". */
const NONE_PRICE = (1n << 128n) - 1n;

export type AuthorityRequirement = 'Autonomous' | 'CoSigned';

export type Action =
  | { kind: 'TopUp'; amount: bigint }
  | { kind: 'PartialClose'; fractionBps: bigint }
  | { kind: 'TakeProfit' }
  | { kind: 'EscalateManualReview' };

export interface ActionCaps {
  topUpUsdPerAction: bigint;
  partialCloseUsdPerAction: bigint;
  dailyTotalUsd: bigint;
}

export interface VenuePolicy {
  maintenanceBps: bigint;
  triggerBufferBps: bigint;
  feeBps: bigint;
  authority: AuthorityRequirement;
  caps: ActionCaps;
  takeProfit: bigint | null;
}

export interface GuardState {
  venue: number;
  venueOwner: Uint8Array;
  coAuthority: Uint8Array;
  authorityReq: AuthorityRequirement;
  policy: VenuePolicy;
  collateral: bigint;
  size: bigint;
  entry: bigint;
  currentPrice: bigint;
  nonce: bigint;
  lastCheckSlot: bigint;
  pending: Action | null;
  pendingIxNonce: bigint | null;
  degraded: boolean;
  staleStreak: number;
  driftMarketIndex: number;
  driftSubaccountId: number;
}

function u128(data: Uint8Array, off: number): bigint {
  let acc = 0n;
  for (let i = 15; i >= 0; i--) acc = (acc << 8n) | BigInt(data[off + i]!);
  return acc;
}

function i128(data: Uint8Array, off: number): bigint {
  const raw = u128(data, off);
  return raw >= 1n << 127n ? raw - (1n << 128n) : raw;
}

function u64(data: Uint8Array, off: number): bigint {
  let acc = 0n;
  for (let i = 7; i >= 0; i--) acc = (acc << 8n) | BigInt(data[off + i]!);
  return acc;
}

function u16(data: Uint8Array, off: number): number {
  return data[off]! | (data[off + 1]! << 8);
}

export function decodeGuardState(data: Uint8Array): GuardState {
  if (data.length !== GUARD_DATA_LEN) {
    throw new Error(`expected ${GUARD_DATA_LEN} bytes, got ${data.length}`);
  }
  if (data[0] !== ACCOUNT_VERSION) {
    throw new Error(`unsupported account version ${data[0]}`);
  }

  const authorityReq: AuthorityRequirement =
    data[G_AUTH_REQ_OFF] === 0 ? 'Autonomous' : 'CoSigned';

  let pending: Action | null;
  switch (data[G_PENDING_TAG_OFF]) {
    case 0:
      pending = null;
      break;
    case 1:
      pending = { kind: 'TopUp', amount: u128(data, G_PENDING_AMT_OFF) };
      break;
    case 2:
      pending = { kind: 'PartialClose', fractionBps: u128(data, G_PENDING_AMT_OFF) };
      break;
    case 3:
      pending = { kind: 'TakeProfit' };
      break;
    case 4:
      pending = { kind: 'EscalateManualReview' };
      break;
    default:
      throw new Error(`unknown pending tag ${data[G_PENDING_TAG_OFF]}`);
  }

  const pxTag = data[G_PX_TAG_OFF];
  if (pxTag !== 0 && pxTag !== 1) throw new Error(`unknown pending_ix tag ${pxTag}`);
  const pendingIxNonce = pxTag === 1 ? u64(data, G_PX_NONCE_OFF) : null;

  const takeProfitRaw = u128(data, G_TP_OFF);

  return {
    venue: data[G_VENUE_OFF]!,
    venueOwner: data.slice(G_VENUE_OWNER_OFF, G_CO_AUTH_OFF),
    coAuthority: data.slice(G_CO_AUTH_OFF, G_AUTH_REQ_OFF),
    authorityReq,
    policy: {
      maintenanceBps: u128(data, G_MAINT_OFF),
      triggerBufferBps: u128(data, G_BUF_OFF),
      feeBps: u128(data, G_FEE_OFF),
      authority: authorityReq,
      caps: {
        topUpUsdPerAction: u128(data, G_CAP_TOP_OFF),
        partialCloseUsdPerAction: u128(data, G_CAP_PARTIAL_OFF),
        dailyTotalUsd: u128(data, G_CAP_DAILY_OFF),
      },
      takeProfit: takeProfitRaw === NONE_PRICE ? null : takeProfitRaw,
    },
    collateral: u128(data, G_COLLAT_OFF),
    size: i128(data, G_SIZE_OFF),
    entry: u128(data, G_ENTRY_OFF),
    currentPrice: u128(data, G_PRICE_OFF),
    nonce: u64(data, G_NONCE_OFF),
    lastCheckSlot: u64(data, G_SLOT_OFF),
    pending,
    pendingIxNonce,
    degraded: data[G_DEGRADED_OFF] === 1,
    staleStreak: data[G_STALE_STREAK_OFF]!,
    driftMarketIndex: u16(data, G_DRIFT_MARKET_OFF),
    driftSubaccountId: u16(data, G_DRIFT_SUBACCOUNT_OFF),
  };
}
