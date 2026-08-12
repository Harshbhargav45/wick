/**
 * `ReconcileVenue` driver — Gap 2's cranker half.
 *
 * The program can compare its model of a position against the venue's own
 * bytes, but only when something hands it the venue account. Nothing did. A
 * position closed or resized at the venue therefore left `state.size` stale
 * indefinitely, and every autonomous order the guard placed was sized from that
 * stale number.
 *
 * This runs on its own cadence, separate from the tick loop, for two reasons:
 * reconciliation is permissionless and cheap, and coupling it to the tick would
 * put a second RPC round-trip on the path the guard's 10s staleness window
 * measures.
 *
 * The nonce is read from the guard (`reconcile_nonce + 1`) rather than counted
 * locally: the program requires it to strictly exceed the stored one, and a
 * local counter desyncs on every restart.
 */
import { PublicKey, Transaction } from "@solana/web3.js";
import { Buffer as IsomorphicBuffer } from "node:buffer";
import {
  G,
  IX,
  RECONCILE_NAMES,
  VENUE_DRIFT,
  readI128LE,
} from "./guard-layout.mjs";
import { driftUserPda } from "./venue-drift.mjs";

const CLOCK_SYSVAR_ADDRESS = "SysvarC1ock11111111111111111111111111111111";

/** Default cadence. Overridden by `RECONCILE_INTERVAL_MS`. */
export const DEFAULT_RECONCILE_INTERVAL_MS = 60_000;

/**
 * Build the `ReconcileVenue` instruction for a Drift guard.
 *
 * Account order mirrors `processor::reconcile_venue`: [0] guard (writable),
 * [1] clock, [2] route_config (readonly), [3] the venue position account.
 */
export function buildReconcileIx({
  programId,
  guard,
  routeConfig,
  venueAccount,
  nonce,
  clockSysvar,
}) {
  const data = IsomorphicBuffer.alloc(9);
  data[0] = IX.ReconcileVenue;
  data.writeBigUInt64LE(BigInt(nonce), 1);
  return {
    programId,
    keys: [
      { pubkey: guard, isSigner: false, isWritable: true },
      { pubkey: clockSysvar, isSigner: false, isWritable: false },
      { pubkey: routeConfig, isSigner: false, isWritable: false },
      { pubkey: venueAccount, isSigner: false, isWritable: false },
    ],
    data,
  };
}

/**
 * Whether this guard is due for reconciliation.
 *
 * Only Drift guards have a venue account the program can decode today, so a
 * Jupiter or venue-less guard is not "overdue" — it is out of scope, and
 * reporting it as skipped every minute would train the operator to ignore the
 * log.
 */
export function reconcileDue(guardData, { now, intervalMs, lastAttemptTs }) {
  if (guardData[G.venue] !== VENUE_DRIFT) return false;
  const intervalSecs = BigInt(Math.floor(intervalMs / 1000));
  const stampedTs = guardData.readBigInt64LE(G.reconcileTs);
  // `lastAttemptTs` covers the case the stamp does not: a reconcile that lands
  // Diverged still stamps `reconcile_ts`, but one that fails to land does not,
  // and without a local memo the loop would retry it every pass.
  const since = lastAttemptTs ?? stampedTs;
  if (since <= 0n) return true; // never reconciled
  return BigInt(now) - since >= intervalSecs;
}

/**
 * Reconcile one guard if it is due. Returns a result object rather than
 * throwing: a reconcile failure must not take the tick loop down with it, and
 * must not be silent either.
 */
export async function maybeReconcile(
  connection,
  {
    programId,
    guard,
    guardData,
    routeConfig,
    payer,
    dryRun,
    now,
    intervalMs = DEFAULT_RECONCILE_INTERVAL_MS,
    lastAttemptTs,
    send,
  }
) {
  if (!reconcileDue(guardData, { now, intervalMs, lastAttemptTs })) {
    return { status: "skipped", reason: "not due" };
  }

  const venueOwner = guardData.subarray(G.venueOwner, G.venueOwner + 32);
  const subAccountId = guardData.readUInt16LE(G.driftSubaccount);
  const venueAccount = driftUserPda(new PublicKey(venueOwner), subAccountId);
  const nonce = guardData.readBigUInt64LE(G.reconcileNonce) + 1n;

  // Refusing here rather than letting the program refuse: `ReconcileVenue`
  // requires the venue account to exist and be Velocity-owned, and a
  // transaction that cannot succeed is not worth a signature or a fee.
  const info = await connection.getAccountInfo(venueAccount);
  if (!info) {
    return {
      status: "unavailable",
      reason: `Drift user ${venueAccount.toBase58()} (sub-account ${subAccountId}) does not exist`,
    };
  }

  const ix = buildReconcileIx({
    programId,
    guard,
    routeConfig,
    venueAccount,
    nonce,
    clockSysvar: new PublicKey(CLOCK_SYSVAR_ADDRESS),
  });

  if (dryRun) {
    return {
      status: "dry-run",
      nonce,
      venueAccount,
      reason: `would reconcile against ${venueAccount.toBase58()} at nonce ${nonce}`,
    };
  }

  try {
    const sig = await send(new Transaction().add(ix), [payer]);
    // Read back the verdict rather than reporting success on a landed
    // transaction: a *diverged* reconcile lands successfully and is the single
    // most important thing the operator needs to hear about, because it blocks
    // autonomous execution until the owner resolves it.
    const after = await connection.getAccountInfo(guard);
    const status = after ? after.data[G.reconcileStatus] : null;
    return {
      status: "landed",
      sig,
      nonce,
      verdict: status === null ? "unknown" : (RECONCILE_NAMES[status] ?? status),
      diverged: status === 2,
      modelSize: readI128LE(guardData, G.size),
      venueSize: after ? readI128LE(after.data, G.venueSize) : null,
    };
  } catch (err) {
    return { status: "failed", reason: err.message };
  }
}

/**
 * One operator-facing line for a reconcile result. A diverged verdict is
 * reported as a failure of the *model*, not of the reconcile — the transaction
 * succeeded, and the guard is now correctly refusing to act on a number the
 * venue contradicts.
 */
export function describeReconcile(guardPubkey, result) {
  const g = guardPubkey.toBase58();
  switch (result.status) {
    case "skipped":
      return null; // not worth a line every pass
    case "dry-run":
      return `[reconcile] dry-run: guard=${g} ${result.reason}`;
    case "unavailable":
      return `[reconcile] guard=${g} cannot reconcile: ${result.reason}`;
    case "failed":
      return `[reconcile] guard=${g} FAILED: ${result.reason}`;
    case "landed":
      return result.diverged
        ? `[reconcile] guard=${g} DIVERGED — model=${result.modelSize} venue=${result.venueSize}; ` +
            `autonomous execution is blocked until the owner runs UpdatePosition`
        : `[reconcile] guard=${g} ${result.verdict} nonce=${result.nonce} sig=${String(result.sig).slice(0, 8)}`;
    default:
      return `[reconcile] guard=${g} ${result.status}`;
  }
}
