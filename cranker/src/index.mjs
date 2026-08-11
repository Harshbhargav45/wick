import {
  ComputeBudgetProgram,
  PublicKey,
  Transaction,
} from "@solana/web3.js";
import { Buffer as IsomorphicBuffer } from "node:buffer";
import { config, cluster, crankerKeypair } from "./config.mjs";
import { fetchLatestVaa } from "./hermes.mjs";
import { buildPostUpdateInstructions } from "./receiver.mjs";
import { logEndpointConfig, sharedConnection, verifyEndpoints } from "./rpc.mjs";
import { routeConfigPda, guardPda } from "./tick.mjs";
import { DELEGATION_PROGRAM_ID } from "./magicblock.mjs";

const CLOCK_SYSVAR = new PublicKey(
  "SysvarC1ock11111111111111111111111111111111"
);
const GUARD_DATA_LEN = 366;
const ACCOUNT_VERSION = 2;

/** Guard byte offsets, mirroring program/src/account.rs. */
const G_VENUE_OWNER_OFF = 2;
const G_NONCE_OFF = 243;

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

/**
 * Bounded runs, matching tick-er.mjs. Without these the loop only ever ends by
 * being killed, so a verification run cannot report its own result: the process
 * has to be terminated from outside, which looks identical to a hang.
 */
const once = process.argv.includes("--once");
const ticksArg = process.argv.indexOf("--ticks");
const maxTicks = once
  ? 1
  : ticksArg > -1
    ? Number(process.argv[ticksArg + 1])
    : Infinity;

/**
 * The version badge is checked client-side rather than with a memcmp filter:
 * the filter takes base58, where "1" is byte 0x00, so version 1 has to be
 * encoded rather than written literally. Filtering here keeps it unambiguous.
 */
async function findGuards(connection, programId) {
  const accounts = await connection.getProgramAccounts(programId, {
    filters: [{ dataSize: GUARD_DATA_LEN }],
  });
  return accounts.filter(
    ({ account }) => account.data[0] === ACCOUNT_VERSION
  );
}

/**
 * Guards this loop *cannot* tick, so the operator is told rather than left to
 * infer it from a shorter list than expected.
 *
 * `findGuards` asks for accounts owned by wick, and a delegated guard is owned
 * by the Delegation Program — so it drops out of that query entirely and this
 * loop reports "tick landed" for the remaining guards while the delegated one
 * silently goes unprotected. Those have to be ticked on the ER instead
 * (`tick-er.mjs`).
 *
 * Ownership is proven by re-deriving `[b"guard", venue_owner]` and checking it
 * matches the account address, rather than trusting length plus a version byte:
 * another program's delegated 366-byte account could otherwise be misreported
 * as a wick guard.
 */
async function findDelegatedGuards(connection) {
  const accounts = await connection.getProgramAccounts(DELEGATION_PROGRAM_ID, {
    filters: [{ dataSize: GUARD_DATA_LEN }],
  });
  return accounts.filter(({ pubkey, account }) => {
    if (account.data[0] !== ACCOUNT_VERSION) return false;
    const owner = new PublicKey(account.data.subarray(2, 34));
    return guardPda(owner).pda.equals(pubkey);
  });
}

async function sendAndConfirm(connection, tx, signers, blockhash) {
  tx.feePayer = config.feePayerPublicKey;
  tx.recentBlockhash =
    blockhash ?? (await connection.getLatestBlockhash("confirmed")).blockhash;
  const sig = await connection.sendTransaction(tx, signers, {
    skipPreflight: true,
    preflightCommitment: "confirmed",
  });
  await connection.confirmTransaction(sig, "confirmed");
  return sig;
}

/**
 * Fire without awaiting confirmation. Rent reclaims are pure cleanup and
 * awaiting them adds two confirmation round-trips to every tick — enough on
 * its own to blow the guard's MAX_TICK_AGE_SECS (10s) heartbeat and flip it to
 * degraded.
 */
async function sendNoWait(connection, tx, signers) {
  const blockhash = (await connection.getLatestBlockhash("confirmed")).blockhash;
  tx.feePayer = config.feePayerPublicKey;
  tx.recentBlockhash = blockhash;
  return connection.sendTransaction(tx, signers, { skipPreflight: true });
}

async function main() {
  const payer = crankerKeypair();
  config.feePayerPublicKey = payer.publicKey;
  const connection = sharedConnection();

  logEndpointConfig(
    { cluster, endpoints: config.rpcEndpoints, rejected: config.rpcRejected },
    (m) => console.log(`[wick-cranker] ${m.replace(/^\[rpc\] /, "")}`)
  );
  console.log(`[wick-cranker] program=${config.wickProgramId.toBase58()}`);
  console.log(
    `[wick-cranker] dryRun=${config.dryRun} interval=${config.tickIntervalMs}ms`
  );

  // An endpoint on the wrong cluster answers every query successfully with data
  // about a chain the guard does not live on, so the loop would report "no guard
  // accounts found" indefinitely. Proving the cluster by genesis hash here turns
  // that into one line at startup. Non-fatal while any endpoint is healthy: the
  // pool rotates past the bad ones.
  const health = await verifyEndpoints({
    endpoints: config.rpcEndpoints,
    expectedCluster: cluster,
  });
  for (const h of health.filter((r) => !r.ok)) {
    console.error(`[wick-cranker] endpoint unusable: ${h.url} — ${h.error}`);
  }
  if (health.every((r) => !r.ok)) {
    console.error(
      `[wick-cranker] no endpoint serves ${cluster}; run 'npm run rpc-check' for detail`
    );
    process.exit(1);
  }

  // Surfaced once at startup, not per tick: a delegated guard is invisible to
  // this loop's own query, so without this the run looks fully successful while
  // that guard goes untouched.
  try {
    const delegated = await findDelegatedGuards(connection);
    for (const { pubkey } of delegated) {
      console.log(
        `[wick-cranker] skipping ${pubkey.toBase58()} — delegated to the MagicBlock ER; tick it with 'node src/tick-er.mjs'`
      );
    }
  } catch (err) {
    console.error(
      `[wick-cranker] could not check for delegated guards: ${err.message}`
    );
  }

  // The guard degrades if consecutive ticks are more than MAX_TICK_AGE_SECS
  // (10s) apart, and a Hermes round-trip alone costs ~3s. Fetching the
  // next VAA while the current tick confirms takes that latency off the
  // critical path instead of adding it to every gap.
  let vaaAhead = fetchLatestVaa().catch(() => null);

  let landed = 0;
  let failed = 0;

  for (let i = 0; i < maxTicks; i += 1) {
    const loopStart = Date.now();
    try {
      const guards = await findGuards(connection, config.wickProgramId);
      if (guards.length === 0) {
        // Counted as a failed tick, not skipped: `continue` here would bypass
        // the interval sleep at the bottom of the loop and spin the RPC.
        failed += 1;
        console.log(
          "[wick-cranker] no guard accounts found on chain; check that the program is deployed and a guard is initialized"
        );
      }

      for (const { pubkey, account } of guards) {
        const { pda: routeConfig } = routeConfigPda();
        const venueOwner = new PublicKey(
          account.data.slice(G_VENUE_OWNER_OFF, G_VENUE_OWNER_OFF + 32)
        );
        const { bump } = guardPda(venueOwner);

        // §8.2 — the guard accepts a tick only at exactly committed + 1, so the
        // nonce is read from the account rather than counted locally. A local
        // counter desyncs across restarts and on every CoSigned tick, where the
        // nonce stays put until the owner confirms.
        const committed = account.data.readBigUInt64LE(G_NONCE_OFF);
        const nonce = committed + 1n;

        const fetched = (await vaaAhead) ?? (await fetchLatestVaa());
        vaaAhead = fetchLatestVaa().catch(() => null);
        const plans = await buildPostUpdateInstructions(fetched.vaa);

        // 1) post the encoded VAA to the Wormhole program
        // 2) post the price update to the Pyth receiver (Full verification) and
        //    drive OnPriceTick in the SAME transaction
        //
        // The guard treats a gap of more than MAX_TICK_AGE_SECS (10s)
        // between ticks as stale and degrades after three. Each awaited
        // confirmation costs slots, so the price post and the tick share one
        // transaction: fewer round-trips, and the tick is guaranteed to read
        // the update it just posted.
        const initTx = new Transaction().add(...plans.initTx);
        const verifyTx = new Transaction().add(...plans.verifyTx);

        const tickData = IsomorphicBuffer.alloc(10);
        tickData[0] = 7; // OnPriceTick discriminator
        tickData.writeBigUInt64LE(nonce, 1);
        tickData[9] = bump;

        const tickIx = {
          programId: config.wickProgramId,
          keys: [
            { pubkey, isSigner: false, isWritable: true },
            { pubkey: CLOCK_SYSVAR, isSigner: false, isWritable: false },
            { pubkey: routeConfig, isSigner: false, isWritable: false },
            { pubkey: plans.priceUpdateAccount, isSigner: false, isWritable: false },
          ],
          data: tickData,
        };

        const postAndTickTx = new Transaction().add(
          ComputeBudgetProgram.setComputeUnitLimit({ units: 250_000 }),
          plans.postUpdateIx,
          tickIx
        );

        if (config.dryRun) {
          console.log(
            `[wick-cranker] dry-run: guard=${pubkey.toBase58()} priceUpdate=${plans.priceUpdateAccount.toBase58()} nonce=${nonce}`
          );
          continue;
        }

        // `initTx` opens the encoded-VAA account, so the new keypair must sign
        // the CreateAccount. `verifyTx` only writes and verifies through the
        // write authority (the payer) — signing it with the VAA keypair too is
        // what produced the "references a signature that is unnecessary" warning.
        // One blockhash for all three: a separate getLatestBlockhash per
        // transaction adds three RPC round-trips to the critical path, and a
        // confirmed blockhash stays valid well past a single tick.
        const blockhash = (await connection.getLatestBlockhash("confirmed"))
          .blockhash;
        await sendAndConfirm(
          connection,
          initTx,
          [payer, plans.encodedVaaSigner],
          blockhash
        );
        await sendAndConfirm(connection, verifyTx, [payer], blockhash);
        const s3 = await sendAndConfirm(
          connection,
          postAndTickTx,
          [payer, plans.priceUpdateSigner],
          blockhash
        );

        // Reclaim the per-tick scratch rent. Fire-and-forget: awaiting these
        // would push the next tick past the guard's staleness window.
        try {
          await sendNoWait(
            connection,
            new Transaction().add(plans.close, plans.reclaimRentIx),
            [payer]
          );
        } catch (e) {
          console.error(`[wick-cranker] reclaim failed (best-effort): ${e.message}`);
        }

        landed += 1;
        console.log(
          `[wick-cranker] tick landed nonce=${nonce} guard=${pubkey.toBase58()} sig=${s3.slice(0, 8)}`
        );
      }
    } catch (err) {
      failed += 1;
      console.error(`[wick-cranker] tick failed: ${err.message}`);
      if (err.logs) console.error(err.logs.join("\n"));
    }
    // Sleep only the remainder of the interval. A fixed sleep would be added on
    // top of a tick that already took longer than the guard's staleness window,
    // guaranteeing the degrade it is meant to avoid.
    if (i + 1 < maxTicks) {
      const spent = Date.now() - loopStart;
      if (spent < config.tickIntervalMs)
        await sleep(config.tickIntervalMs - spent);
    }
  }

  if (Number.isFinite(maxTicks)) {
    console.log(`[wick-cranker] done: ${landed} landed, ${failed} failed`);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
