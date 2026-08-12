/**
 * `ReconcileVenue` driver — Gap 2's cranker half.
 *
 * The two properties that matter: the nonce comes from the account (a local
 * counter desyncs across restarts and the program requires strict increase), and
 * a *diverged* verdict is reported as a loud, distinguishable outcome. A
 * diverged reconcile lands successfully on chain, so a driver that reports
 * "landed" and stops there hides the one state that blocks autonomous
 * protection.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { PublicKey } from "@solana/web3.js";
import {
  ACCOUNT_VERSION,
  G,
  GUARD_DATA_LEN,
  IX,
  RECONCILE_CONVERGED,
  RECONCILE_DIVERGED,
  VENUE_DRIFT,
  VENUE_JUPITER,
  writeU128LE,
} from "../src/guard-layout.mjs";
import {
  buildReconcileIx,
  describeReconcile,
  maybeReconcile,
  reconcileDue,
} from "../src/reconcile.mjs";
import { driftUserPda } from "../src/venue-drift.mjs";

const PROGRAM_ID = new PublicKey("11111111111111111111111111111112");
const GUARD = new PublicKey("11111111111111111111111111111113");
const ROUTE_CONFIG = new PublicKey("11111111111111111111111111111114");
const VENUE_OWNER = new PublicKey("11111111111111111111111111111115");
const PAYER = { publicKey: new PublicKey("11111111111111111111111111111116") };
const DRIFT_PROGRAM_ID = new PublicKey("vELoC1audYbSYVRXn1vPaV8Axoa9oU6BYmNGZZBDZ1P");

const NOW = 1_700_000_000;

function guardData(over = {}) {
  const d = Buffer.alloc(GUARD_DATA_LEN);
  d[G.version] = ACCOUNT_VERSION;
  d[G.venue] = over.venue ?? VENUE_DRIFT;
  VENUE_OWNER.toBuffer().copy(d, G.venueOwner);
  writeU128LE(d, over.size ?? 100_000_000n, G.size);
  d.writeUInt16LE(over.subaccount ?? 0, G.driftSubaccount);
  d.writeBigInt64LE(BigInt(over.reconcileTs ?? 0), G.reconcileTs);
  d.writeBigUInt64LE(BigInt(over.reconcileNonce ?? 0), G.reconcileNonce);
  d[G.reconcileStatus] = over.reconcileStatus ?? 0;
  return d;
}

/** Guard data as it looks *after* a reconcile lands, for the read-back. */
function afterData(status, venueSize) {
  const d = guardData({ reconcileStatus: status });
  writeU128LE(d, venueSize, G.venueSize);
  return d;
}

function fakeConnection({ venueExists = true, after = null } = {}) {
  return {
    async getAccountInfo(key) {
      if (after && key.equals(GUARD)) return { data: after, owner: PROGRAM_ID };
      if (!venueExists) return null;
      return { owner: DRIFT_PROGRAM_ID, data: Buffer.alloc(4496) };
    },
  };
}

const BASE = {
  programId: PROGRAM_ID,
  guard: GUARD,
  routeConfig: ROUTE_CONFIG,
  payer: PAYER,
  dryRun: false,
  now: NOW,
  intervalMs: 60_000,
};

test("the instruction carries discriminator 13 and the nonce, in that order", () => {
  const ix = buildReconcileIx({
    programId: PROGRAM_ID,
    guard: GUARD,
    routeConfig: ROUTE_CONFIG,
    venueAccount: VENUE_OWNER,
    nonce: 7n,
    clockSysvar: ROUTE_CONFIG,
  });
  assert.equal(ix.data.length, 9, "the program rejects any other payload length");
  assert.equal(ix.data[0], IX.ReconcileVenue);
  assert.equal(ix.data[0], 13);
  assert.equal(ix.data.readBigUInt64LE(1), 7n);
});

test("the account order matches processor::reconcile_venue", () => {
  const ix = buildReconcileIx({
    programId: PROGRAM_ID,
    guard: GUARD,
    routeConfig: ROUTE_CONFIG,
    venueAccount: VENUE_OWNER,
    nonce: 1n,
    clockSysvar: GUARD,
  });
  assert.equal(ix.keys.length, 4);
  assert.equal(ix.keys[0].pubkey.toBase58(), GUARD.toBase58());
  assert.equal(ix.keys[0].isWritable, true, "the guard is written");
  assert.equal(ix.keys[2].pubkey.toBase58(), ROUTE_CONFIG.toBase58());
  assert.equal(ix.keys[3].pubkey.toBase58(), VENUE_OWNER.toBase58());
  assert.equal(ix.keys[3].isWritable, false, "the venue account is only read");
  // Permissionless: reconciliation needs no signature. The caller chooses when
  // the guard looks at the venue, never what it sees.
  assert.ok(ix.keys.every((k) => !k.isSigner));
});

test("a guard that has never reconciled is due immediately", () => {
  assert.equal(reconcileDue(guardData(), { now: NOW, intervalMs: 60_000 }), true);
});

test("cadence is respected, and measured in seconds not milliseconds", () => {
  const d = guardData({ reconcileTs: NOW - 30 });
  assert.equal(reconcileDue(d, { now: NOW, intervalMs: 60_000 }), false);
  assert.equal(reconcileDue(d, { now: NOW + 31, intervalMs: 60_000 }), true);
});

test("only Drift guards are due — others are out of scope, not overdue", () => {
  for (const venue of [0, VENUE_JUPITER]) {
    assert.equal(
      reconcileDue(guardData({ venue }), { now: NOW, intervalMs: 60_000 }),
      false,
      `venue ${venue} has no venue account the program can decode`
    );
  }
});

test("a local attempt memo suppresses retry when the on-chain stamp cannot move", () => {
  // A reconcile that fails to land never stamps `reconcile_ts`, so without the
  // memo a broken guard is retried on every pass forever.
  const d = guardData({ reconcileTs: 0 });
  assert.equal(
    reconcileDue(d, { now: NOW, intervalMs: 60_000, lastAttemptTs: BigInt(NOW - 5) }),
    false
  );
  assert.equal(
    reconcileDue(d, { now: NOW, intervalMs: 60_000, lastAttemptTs: BigInt(NOW - 61) }),
    true
  );
});

test("the nonce comes from the account, strictly incremented", async () => {
  let sent = null;
  const result = await maybeReconcile(fakeConnection({ after: afterData(RECONCILE_CONVERGED, 100_000_000n) }), {
    ...BASE,
    guardData: guardData({ reconcileNonce: 41 }),
    send: async (tx) => {
      sent = tx;
      return "sig11111";
    },
  });
  assert.equal(result.status, "landed");
  assert.equal(result.nonce, 42n, "the program requires strict increase over 41");
  assert.equal(sent.instructions[0].data.readBigUInt64LE(1), 42n);
});

test("the venue account is the guard's own sub-account PDA", async () => {
  let sent = null;
  await maybeReconcile(fakeConnection({ after: afterData(RECONCILE_CONVERGED, 1n) }), {
    ...BASE,
    guardData: guardData({ subaccount: 3 }),
    send: async (tx) => {
      sent = tx;
      return "sig";
    },
  });
  assert.equal(
    sent.instructions[0].keys[3].pubkey.toBase58(),
    driftUserPda(VENUE_OWNER, 3).toBase58()
  );
});

test("a diverged verdict is surfaced as diverged, not as success", async () => {
  const result = await maybeReconcile(
    fakeConnection({ after: afterData(RECONCILE_DIVERGED, 20_000_000n) }),
    { ...BASE, guardData: guardData(), send: async () => "sigDIVERGE" }
  );
  // The transaction succeeded — that is exactly why reporting only the
  // transaction result would hide this.
  assert.equal(result.status, "landed");
  assert.equal(result.diverged, true);
  assert.equal(result.verdict, "DIVERGED");
  assert.equal(result.modelSize, 100_000_000n);
  assert.equal(result.venueSize, 20_000_000n);

  const line = describeReconcile(GUARD, result);
  assert.match(line, /DIVERGED/);
  assert.match(line, /model=100000000 venue=20000000/);
  assert.match(line, /autonomous execution is blocked/);
});

test("a converged verdict reads as converged", async () => {
  const result = await maybeReconcile(
    fakeConnection({ after: afterData(RECONCILE_CONVERGED, 100_000_000n) }),
    { ...BASE, guardData: guardData(), send: async () => "sigOK" }
  );
  assert.equal(result.diverged, false);
  assert.equal(result.verdict, "converged");
  assert.doesNotMatch(describeReconcile(GUARD, result), /DIVERGED/);
});

test("a missing venue account costs no signature and no fee", async () => {
  let sendCalled = false;
  const result = await maybeReconcile(fakeConnection({ venueExists: false }), {
    ...BASE,
    guardData: guardData({ subaccount: 9 }),
    send: async () => {
      sendCalled = true;
      return "sig";
    },
  });
  assert.equal(result.status, "unavailable");
  assert.match(result.reason, /sub-account 9/);
  assert.equal(sendCalled, false, "a transaction that cannot succeed was sent");
  assert.match(describeReconcile(GUARD, result), /cannot reconcile/);
});

test("a dry run builds but never sends", async () => {
  let sendCalled = false;
  const result = await maybeReconcile(fakeConnection(), {
    ...BASE,
    dryRun: true,
    guardData: guardData({ reconcileNonce: 4 }),
    send: async () => {
      sendCalled = true;
      return "sig";
    },
  });
  assert.equal(result.status, "dry-run");
  assert.equal(result.nonce, 5n);
  assert.equal(sendCalled, false);
});

test("a send failure is reported, not swallowed", async () => {
  const result = await maybeReconcile(fakeConnection(), {
    ...BASE,
    guardData: guardData(),
    send: async () => {
      throw new Error("blockhash not found");
    },
  });
  assert.equal(result.status, "failed");
  assert.match(result.reason, /blockhash not found/);
  assert.match(describeReconcile(GUARD, result), /FAILED/);
});

test("a guard that is not due is skipped without a log line", async () => {
  const result = await maybeReconcile(fakeConnection(), {
    ...BASE,
    guardData: guardData({ reconcileTs: NOW - 1 }),
    send: async () => {
      throw new Error("must not send");
    },
  });
  assert.equal(result.status, "skipped");
  // Silence is correct here: a line every pass for every healthy guard trains
  // the operator to ignore the log.
  assert.equal(describeReconcile(GUARD, result), null);
});
