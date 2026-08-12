/**
 * Wire-format decoders for the program's three account types.
 *
 * This mirrors `GuardState::from_bytes` and `WalletState::from_bytes` in
 * `program/src/account.rs` byte for byte, and is kept in step with
 * `cranker/src/guard-layout.mjs`, which is the same table for the off-chain
 * crank. If the on-chain layout changes, all three move together.
 *
 * v3 appended the venue snapshot the reconciler writes, its verdict, and the
 * margin-reserve bump. There is no v2 compatibility path: `ACCOUNT_VERSION` is
 * one constant shared by the guard, the route config and the margin reserve, so
 * the bump invalidated all three at once and the program rejects the old
 * encoding outright. Decoding a v2 account here would be decoding something the
 * chain no longer accepts.
 */

export const ACCOUNT_VERSION = 3;
export const GUARD_DATA_LEN = 416;
export const WALLET_DATA_LEN = 81;
export const PENDING_IX_DATA_LEN = 50;

export const SCALE = 1_000_000n;
export const BPS_DENOM = 10_000n;

/** No venue adapter: the guard watches and records but never dispatches. */
export const VENUE_NONE = 0;
export const VENUE_JUPITER = 2;
export const VENUE_DRIFT = 3;

/** `reconcile_status`, mirroring `account.rs`. */
export const RECONCILE_NEVER = 0;
export const RECONCILE_CONVERGED = 1;
export const RECONCILE_DIVERGED = 2;

/** `pending_ix` kinds, mirroring `account.rs`. */
export const PENDING_IX_NONE = 0;
export const PENDING_IX_JUPITER_TPSL = 1;
export const PENDING_IX_JUPITER_DEFENSIVE_CLOSE = 2;

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
const G_LAST_CHECK_OFF = 251;
const G_PENDING_TAG_OFF = 259;
const G_PENDING_AMT_OFF = 260;
const G_DEGRADED_OFF = 276;
const G_STALE_STREAK_OFF = 277;
const G_PX_TAG_OFF = 278;
const G_PX_NONCE_OFF = 279;
const G_DRIFT_MARKET_OFF = 338;
const G_DRIFT_SUBACCOUNT_OFF = 340;
const G_DAILY_SPENT_OFF = 342;
const G_DAILY_EPOCH_OFF = 358;
// v3.
const G_VENUE_SIZE_OFF = 366;
const G_VENUE_COLLAT_OFF = 382;
const G_RECONCILE_TS_OFF = 398;
const G_RECONCILE_NONCE_OFF = 406;
const G_RECONCILE_STATUS_OFF = 414;
const G_MARGIN_BUMP_OFF = 415;

const W_OWNER_OFF = 1;
const W_CO_AUTH_OFF = 33;
const W_BALANCE_OFF = 65;

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

/**
 * What the venue's own account said at the last `ReconcileVenue`, and whether
 * the guard's model agreed with it.
 *
 * `status === RECONCILE_DIVERGED` is a fail-closed state on-chain: autonomous
 * execution is refused until the snapshot converges again. The console has to
 * surface it as unhealthy, because by every other measure — collateral, price,
 * staleness — a diverged guard looks fine while protecting a position that is
 * not the one it thinks it is watching.
 */
export interface ReconcileState {
  status: number;
  /** Unix seconds of the last reconcile; 0 when it has never run. */
  ts: bigint;
  /** Guard nonce at the last reconcile, for ordering against `nonce`. */
  nonce: bigint;
  /** The venue's size, signed. Meaningless while `status` is NEVER. */
  venueSize: bigint;
  /** The venue's collateral. Meaningless while `status` is NEVER. */
  venueCollateral: bigint;
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
  /**
   * Unix seconds of the last accepted tick, signed because the program stores
   * an `i64` and 0 means "never ticked". Not a slot: the program compares it
   * against `MAX_TICK_AGE_SECS` on the wall clock, because slot length drifts.
   */
  lastCheckTs: bigint;
  pending: Action | null;
  /** Kind of owner-signed venue instruction staged, if any. */
  pendingIxKind: number;
  pendingIxNonce: bigint | null;
  degraded: boolean;
  staleStreak: number;
  driftMarketIndex: number;
  driftSubaccountId: number;
  /** USD (6dp) already committed by guard actions in the current daily epoch. */
  dailySpentUsd: bigint;
  /** Unix seconds the current epoch began; rolls over after DAILY_EPOCH_SECS. */
  dailyEpochStartTs: bigint;
  reconcile: ReconcileState;
  /**
   * Bump of the guard's margin reserve, or `null` when no reserve exists yet.
   *
   * The program writes the bump at `InitMarginWallet` and reads it to derive
   * the reserve during an autonomous top-up, so `null` here is exactly the
   * condition under which a `TopUp` action has no lamports behind it.
   */
  marginWalletBump: number | null;
}

/** The 2-of-2 margin reserve that backs `TopUp` with real lamports. */
export interface WalletState {
  owner: Uint8Array;
  coAuthority: Uint8Array;
  /**
   * Lamports credited to the reserve, which is what the program will spend.
   * The account also holds its rent-exempt minimum on top of this, and that
   * part is not withdrawable.
   */
  balance: bigint;
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

function i64(data: Uint8Array, off: number): bigint {
  const raw = u64(data, off);
  return raw >= 1n << 63n ? raw - (1n << 64n) : raw;
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

  const pendingIxKind = data[G_PX_TAG_OFF]!;
  if (pendingIxKind > PENDING_IX_JUPITER_DEFENSIVE_CLOSE) {
    throw new Error(`unknown pending_ix kind ${pendingIxKind}`);
  }
  const pendingIxNonce =
    pendingIxKind === PENDING_IX_NONE ? null : u64(data, G_PX_NONCE_OFF);

  const reconcileStatus = data[G_RECONCILE_STATUS_OFF]!;
  if (reconcileStatus > RECONCILE_DIVERGED) {
    throw new Error(`unknown reconcile status ${reconcileStatus}`);
  }

  const takeProfitRaw = u128(data, G_TP_OFF);
  const marginBump = data[G_MARGIN_BUMP_OFF]!;

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
    lastCheckTs: i64(data, G_LAST_CHECK_OFF),
    pending,
    pendingIxKind,
    pendingIxNonce,
    degraded: data[G_DEGRADED_OFF] === 1,
    staleStreak: data[G_STALE_STREAK_OFF]!,
    driftMarketIndex: u16(data, G_DRIFT_MARKET_OFF),
    driftSubaccountId: u16(data, G_DRIFT_SUBACCOUNT_OFF),
    dailySpentUsd: u128(data, G_DAILY_SPENT_OFF),
    dailyEpochStartTs: i64(data, G_DAILY_EPOCH_OFF),
    reconcile: {
      status: reconcileStatus,
      ts: i64(data, G_RECONCILE_TS_OFF),
      nonce: u64(data, G_RECONCILE_NONCE_OFF),
      venueSize: i128(data, G_VENUE_SIZE_OFF),
      venueCollateral: u128(data, G_VENUE_COLLAT_OFF),
    },
    // 0 is not a reachable bump for a PDA — `find_program_address` counts down
    // from 255 and a seed that only canonicalizes at 0 would have had to fail
    // 255 curve checks first — so the program uses it as "no reserve linked".
    marginWalletBump: marginBump === 0 ? null : marginBump,
  };
}

export function decodeWalletState(data: Uint8Array): WalletState {
  if (data.length !== WALLET_DATA_LEN) {
    throw new Error(`expected ${WALLET_DATA_LEN} bytes, got ${data.length}`);
  }
  if (data[0] !== ACCOUNT_VERSION) {
    throw new Error(`unsupported account version ${data[0]}`);
  }

  return {
    owner: data.slice(W_OWNER_OFF, W_CO_AUTH_OFF),
    coAuthority: data.slice(W_CO_AUTH_OFF, W_BALANCE_OFF),
    balance: u128(data, W_BALANCE_OFF),
  };
}
