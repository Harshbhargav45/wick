/**
 * Drift (Velocity) venue accounts for `OnPriceTick` — Gap 4.
 *
 * A Drift guard is the autonomous tier: on a breach the program CPIs into
 * `place_perp_order` itself. That CPI needs Velocity's fixed `state`/`user`/
 * `authority` trio plus the oracle and market accounts, and the guard can only
 * use accounts the *transaction* carries. Ticking a Drift guard with the bare
 * four-account layout therefore produces a guard that computes the right
 * reduce and then fails `InvalidInstruction` at the adapter — protection that
 * exists everywhere except at the moment it is needed.
 *
 * This module builds accounts `[4..]`. The program reads them as:
 *
 *   [4] state      (readonly)
 *   [5] user       (writable)  PDA ["user", venue_owner, sub_account_id u16 LE]
 *   [6] authority  (readonly, non-signer here — the *program* signs as the
 *                   guard PDA via `invoke_signed`; a signer flag on an address
 *                   no keypair in this transaction controls would make the
 *                   whole transaction unsignable)
 *   [7..] remaining: oracles, then spot markets, then perp markets — the order
 *                   Drift's own `getRemainingAccounts` emits.
 *
 * Capped at `MAX_DRIFT_ACCOUNTS` (16) total, matching `drift.rs`: past that the
 * adapter rejects the tick, so overflowing is a loud failure here rather than a
 * confusing one on chain.
 */
import { PublicKey } from "@solana/web3.js";
import { Buffer as IsomorphicBuffer } from "node:buffer";

/** Velocity program id — `drift.rs::DRIFT_PROGRAM_ID`. */
export const DRIFT_PROGRAM_ID = new PublicKey(
  "vELoC1audYbSYVRXn1vPaV8Axoa9oU6BYmNGZZBDZ1P"
);

/** `drift.rs::MAX_DRIFT_ACCOUNTS` — the adapter's own ceiling. */
export const MAX_DRIFT_ACCOUNTS = 16;

/** Offsets into Velocity's `PerpMarket` / `SpotMarket`, per `drift.rs`. */
const PERP_ORACLE_OFF = 312;
const SPOT_ORACLE_OFF = 40;

const PERP_MARKET_SEED = "perp_market";
const SPOT_MARKET_SEED = "spot_market";
const USER_SEED = "user";
const STATE_SEED = "drift_state";

/**
 * Quote spot market index. Market 0 is USDC on both Drift and Velocity; the
 * quote market's oracle is part of the margin calculation for any perp, so the
 * CPI needs it even for a pure perp reduce.
 */
const QUOTE_SPOT_MARKET_INDEX = 0;

function u16le(n) {
  const b = IsomorphicBuffer.alloc(2);
  b.writeUInt16LE(n, 0);
  return b;
}

export function driftStatePda() {
  return PublicKey.findProgramAddressSync(
    [IsomorphicBuffer.from(STATE_SEED)],
    DRIFT_PROGRAM_ID
  )[0];
}

export function driftUserPda(venueOwner, subAccountId) {
  return PublicKey.findProgramAddressSync(
    [IsomorphicBuffer.from(USER_SEED), venueOwner.toBuffer(), u16le(subAccountId)],
    DRIFT_PROGRAM_ID
  )[0];
}

export function perpMarketPda(marketIndex) {
  return PublicKey.findProgramAddressSync(
    [IsomorphicBuffer.from(PERP_MARKET_SEED), u16le(marketIndex)],
    DRIFT_PROGRAM_ID
  )[0];
}

export function spotMarketPda(marketIndex) {
  return PublicKey.findProgramAddressSync(
    [IsomorphicBuffer.from(SPOT_MARKET_SEED), u16le(marketIndex)],
    DRIFT_PROGRAM_ID
  )[0];
}

/**
 * Read the oracle a market points at, rather than deriving or assuming one.
 *
 * Drift markets are re-pointed at new oracle accounts (feed migrations, the
 * switchboard→pull moves), and an oracle guessed from a hard-coded table is
 * exactly the account that silently goes stale. The market account is the
 * authority on its own oracle, so it is the only honest source.
 */
export function readOracle(marketData, offset) {
  if (!marketData || marketData.length < offset + 32) {
    throw new Error(
      `market account too short to hold an oracle at ${offset} (len=${marketData?.length})`
    );
  }
  const oracle = new PublicKey(marketData.subarray(offset, offset + 32));
  if (oracle.equals(PublicKey.default)) {
    throw new Error(`market has no oracle configured at offset ${offset}`);
  }
  return oracle;
}

/**
 * Build the `[4..]` tail for a Drift guard's tick.
 *
 * Returns `{ keys, accounts }` where `keys` are `AccountMeta`s ready to append,
 * and `accounts` names each one for logging. Throws with a specific reason when
 * the accounts cannot be assembled — the caller turns that into a loud skip for
 * that guard rather than a tick that will fail on chain.
 */
export async function buildDriftTickAccounts(connection, {
  guardPda,
  venueOwner,
  marketIndex,
  subAccountId,
}) {
  const state = driftStatePda();
  const user = driftUserPda(venueOwner, subAccountId);
  const perpMarket = perpMarketPda(marketIndex);
  const spotMarket = spotMarketPda(QUOTE_SPOT_MARKET_INDEX);

  // One batched fetch: three sequential round-trips per guard per tick is
  // enough on its own to push a 5s loop past MAX_TICK_AGE_SECS (10s).
  const [userInfo, perpInfo, spotInfo] = await connection.getMultipleAccountsInfo(
    [user, perpMarket, spotMarket]
  );

  if (!userInfo) {
    throw new Error(
      `Drift user account ${user.toBase58()} does not exist — the venue owner has no sub-account ${subAccountId}`
    );
  }
  if (!userInfo.owner.equals(DRIFT_PROGRAM_ID)) {
    throw new Error(
      `Drift user account ${user.toBase58()} is owned by ${userInfo.owner.toBase58()}, not Velocity`
    );
  }
  if (!perpInfo) {
    throw new Error(
      `perp market ${marketIndex} (${perpMarket.toBase58()}) does not exist on this cluster`
    );
  }
  if (!spotInfo) {
    throw new Error(
      `quote spot market ${QUOTE_SPOT_MARKET_INDEX} (${spotMarket.toBase58()}) does not exist on this cluster`
    );
  }

  const perpOracle = readOracle(perpInfo.data, PERP_ORACLE_OFF);
  const spotOracle = readOracle(spotInfo.data, SPOT_ORACLE_OFF);

  // Oracles, then spot markets, then perp markets — the order Drift's own
  // `getRemainingAccounts` emits. Deviating from it does not fail loudly; the
  // program reads the wrong account as the wrong type.
  const remaining = [
    { pubkey: perpOracle, isSigner: false, isWritable: false, name: "perpOracle" },
    { pubkey: spotOracle, isSigner: false, isWritable: false, name: "spotOracle" },
    { pubkey: spotMarket, isSigner: false, isWritable: false, name: "spotMarket" },
    { pubkey: perpMarket, isSigner: false, isWritable: true, name: "perpMarket" },
  ];

  const keys = [
    { pubkey: state, isSigner: false, isWritable: false, name: "state" },
    { pubkey: user, isSigner: false, isWritable: true, name: "user" },
    // Non-signer: the guard PDA has no keypair. `place_perp_order` accepts it
    // because the program CPIs with `invoke_signed` over the guard's own seeds,
    // and the venue owner registered the guard as the sub-account's delegate.
    { pubkey: guardPda, isSigner: false, isWritable: false, name: "authority" },
    ...remaining,
  ];

  if (keys.length > MAX_DRIFT_ACCOUNTS) {
    throw new Error(
      `venue account list is ${keys.length}, over the adapter's ceiling of ${MAX_DRIFT_ACCOUNTS}`
    );
  }

  // Duplicate accounts are legal on Solana but here they mean a derivation
  // collapsed — two seeds producing one address would have the CPI read the
  // same account as two different market types.
  const seen = new Set(keys.map((k) => k.pubkey.toBase58()));
  if (seen.size !== keys.length) {
    throw new Error("venue account list contains duplicates — a derivation collapsed");
  }

  return {
    keys: keys.map(({ pubkey, isSigner, isWritable }) => ({
      pubkey,
      isSigner,
      isWritable,
    })),
    accounts: keys.map(({ name, pubkey }) => `${name}=${pubkey.toBase58()}`),
  };
}

/**
 * One-line operator-facing reason a guard could not be ticked with venue
 * accounts. Kept separate so the loop logs a cause, never a bare stack.
 */
export function describeVenueAccountError(err) {
  return `venue accounts unavailable: ${err.message}`;
}
