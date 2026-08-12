/**
 * Drift venue-account assembly — Gap 4.
 *
 * The property under test is not "these functions return something" but that a
 * Drift guard's tick carries accounts the on-chain adapter will accept, in the
 * order it reads them, and that anything else fails loudly here rather than
 * on chain. A tick that lands without venue accounts is the worst outcome
 * available: the guard reports healthy operation right up to the breach, then
 * fails at the adapter.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { PublicKey } from "@solana/web3.js";
import {
  DRIFT_PROGRAM_ID,
  MAX_DRIFT_ACCOUNTS,
  buildDriftTickAccounts,
  driftUserPda,
  perpMarketPda,
  readOracle,
  spotMarketPda,
} from "../src/venue-drift.mjs";

const PERP_ORACLE = new PublicKey("H6ARHf6YXhGYeQfUzQNGk6rDNnLBQKrenN712K4AQJEG");
const SPOT_ORACLE = new PublicKey("Gnt27xtC473ZT2Mw5u8wZ68Z3gULkSTb5DuxJy7eJotD");
const GUARD = new PublicKey("11111111111111111111111111111112");
const VENUE_OWNER = new PublicKey("11111111111111111111111111111113");

/** A market account whose oracle sits at `offset`, as the real ones do. */
function marketAccount(oracle, offset, len = 1200) {
  const data = Buffer.alloc(len);
  oracle.toBuffer().copy(data, offset);
  return { owner: DRIFT_PROGRAM_ID, data };
}

function userAccount(owner = DRIFT_PROGRAM_ID) {
  return { owner, data: Buffer.alloc(4496) };
}

/**
 * A connection stub. Only `getMultipleAccountsInfo` is used, so anything else
 * being called is itself a failure worth surfacing.
 */
function fakeConnection(responses) {
  return {
    async getMultipleAccountsInfo() {
      return responses;
    },
  };
}

const OK_RESPONSES = [
  userAccount(),
  marketAccount(PERP_ORACLE, 312),
  marketAccount(SPOT_ORACLE, 40),
];

const ARGS = {
  guardPda: GUARD,
  venueOwner: VENUE_OWNER,
  marketIndex: 0,
  subAccountId: 0,
};

test("PDAs derive under the Velocity program, not wick", async () => {
  for (const pda of [
    driftUserPda(VENUE_OWNER, 0),
    perpMarketPda(0),
    spotMarketPda(0),
  ]) {
    assert.ok(pda instanceof PublicKey);
  }
  // The sub-account id is part of the seed, so two sub-accounts must not
  // collapse onto one user account.
  assert.notEqual(
    driftUserPda(VENUE_OWNER, 0).toBase58(),
    driftUserPda(VENUE_OWNER, 1).toBase58()
  );
  assert.notEqual(perpMarketPda(0).toBase58(), perpMarketPda(1).toBase58());
});

test("the account list is in the order the adapter reads it", async () => {
  const built = await buildDriftTickAccounts(fakeConnection(OK_RESPONSES), ARGS);

  // [4] state, [5] user, [6] authority — the fixed trio.
  assert.equal(built.keys[1].pubkey.toBase58(), driftUserPda(VENUE_OWNER, 0).toBase58());
  assert.equal(built.keys[2].pubkey.toBase58(), GUARD.toBase58());

  // The user account is the only writable in the fixed trio: `place_perp_order`
  // mutates the sub-account's order list.
  assert.equal(built.keys[0].isWritable, false, "state must be readonly");
  assert.equal(built.keys[1].isWritable, true, "user must be writable");

  // Oracles, then spot market, then perp market — Drift's own
  // getRemainingAccounts order.
  assert.deepEqual(
    built.keys.slice(3).map((k) => k.pubkey.toBase58()),
    [
      PERP_ORACLE.toBase58(),
      SPOT_ORACLE.toBase58(),
      spotMarketPda(0).toBase58(),
      perpMarketPda(0).toBase58(),
    ]
  );
});

test("the authority is not marked a signer", async () => {
  // The guard PDA has no keypair. A signer flag here makes the whole
  // transaction unsignable — the program signs the CPI itself via
  // invoke_signed over the guard's seeds.
  const built = await buildDriftTickAccounts(fakeConnection(OK_RESPONSES), ARGS);
  assert.equal(built.keys[2].isSigner, false);
  assert.ok(built.keys.every((k) => k.isSigner === false));
});

test("the list stays inside the adapter's 16-account ceiling", async () => {
  const built = await buildDriftTickAccounts(fakeConnection(OK_RESPONSES), ARGS);
  // +4 for guard/clock/routeConfig/priceUpdate ahead of these.
  assert.ok(
    built.keys.length + 4 <= MAX_DRIFT_ACCOUNTS,
    `${built.keys.length} + 4 exceeds ${MAX_DRIFT_ACCOUNTS}`
  );
});

test("the oracle is read from the market, not guessed", async () => {
  // Re-pointing the market at a different oracle must change the account list.
  const other = new PublicKey("11111111111111111111111111111114");
  const built = await buildDriftTickAccounts(
    fakeConnection([userAccount(), marketAccount(other, 312), marketAccount(SPOT_ORACLE, 40)]),
    ARGS
  );
  assert.equal(built.keys[3].pubkey.toBase58(), other.toBase58());
});

test("a market with no oracle configured is refused", async () => {
  await assert.rejects(
    buildDriftTickAccounts(
      fakeConnection([userAccount(), marketAccount(PublicKey.default, 312), marketAccount(SPOT_ORACLE, 40)]),
      ARGS
    ),
    /no oracle configured/
  );
});

test("a truncated market account is refused rather than read past its end", () => {
  assert.throws(() => readOracle(Buffer.alloc(100), 312), /too short/);
  assert.throws(() => readOracle(undefined, 312), /too short/);
});

test("a missing Drift sub-account is refused by name", async () => {
  await assert.rejects(
    buildDriftTickAccounts(
      fakeConnection([null, marketAccount(PERP_ORACLE, 312), marketAccount(SPOT_ORACLE, 40)]),
      { ...ARGS, subAccountId: 4 }
    ),
    /does not exist — the venue owner has no sub-account 4/
  );
});

test("a user account owned by something other than Velocity is refused", async () => {
  // The single most dangerous input: right address, right length, wrong
  // program. Reading it as a Drift `User` decodes another program's bytes as a
  // position.
  await assert.rejects(
    buildDriftTickAccounts(
      fakeConnection([
        userAccount(new PublicKey("11111111111111111111111111111115")),
        marketAccount(PERP_ORACLE, 312),
        marketAccount(SPOT_ORACLE, 40),
      ]),
      ARGS
    ),
    /not Velocity/
  );
});

test("a missing market is refused with the index that is missing", async () => {
  await assert.rejects(
    buildDriftTickAccounts(
      fakeConnection([userAccount(), null, marketAccount(SPOT_ORACLE, 40)]),
      { ...ARGS, marketIndex: 5 }
    ),
    /perp market 5 .* does not exist/
  );
  await assert.rejects(
    buildDriftTickAccounts(
      fakeConnection([userAccount(), marketAccount(PERP_ORACLE, 312), null]),
      ARGS
    ),
    /quote spot market 0 .* does not exist/
  );
});

test("a derivation collapse is caught rather than passed to the CPI", async () => {
  // If the perp and spot oracles resolved to the same account, the CPI would
  // read one account as two different market types.
  await assert.rejects(
    buildDriftTickAccounts(
      fakeConnection([
        userAccount(),
        marketAccount(PERP_ORACLE, 312),
        marketAccount(PERP_ORACLE, 40),
      ]),
      ARGS
    ),
    /duplicates/
  );
});

test("a non-zero market index reaches the derivation", async () => {
  const built = await buildDriftTickAccounts(fakeConnection(OK_RESPONSES), {
    ...ARGS,
    marketIndex: 3,
  });
  assert.equal(
    built.keys.at(-1).pubkey.toBase58(),
    perpMarketPda(3).toBase58(),
    "market index is ignored — every guard would trade market 0"
  );
});
