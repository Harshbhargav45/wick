import {
  ComputeBudgetProgram,
  Connection,
  PublicKey,
  Transaction,
} from "@solana/web3.js";
import { Buffer as IsomorphicBuffer } from "node:buffer";
import { config, crankerKeypair } from "./config.mjs";
import { fetchLatestVaa } from "./hermes.mjs";
import { buildPostUpdateInstructions } from "./receiver.mjs";
import { routeConfigPda, guardPda } from "./tick.mjs";

const CLOCK_SYSVAR = new PublicKey(
  "SysvarC1ock11111111111111111111111111111111"
);
const GUARD_DATA_LEN = 342;
const ACCOUNT_VERSION = 1;

/** Guard byte offsets, mirroring program/src/account.rs. */
const G_VENUE_OWNER_OFF = 2;
const G_NONCE_OFF = 243;

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

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
 * its own to blow the guard's 25-slot heartbeat and flip it to degraded.
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
  const connection = new Connection(config.rpc, "confirmed");

  console.log(
    `[wick-cranker] rpc=${config.rpc} program=${config.wickProgramId.toBase58()}`
  );
  console.log(
    `[wick-cranker] dryRun=${config.dryRun} interval=${config.tickIntervalMs}ms`
  );

  // The guard degrades if consecutive ticks are more than MAX_TICK_AGE_SLOTS
  // (25, ~10s) apart, and a Hermes round-trip alone costs ~3s. Fetching the
  // next VAA while the current tick confirms takes that latency off the
  // critical path instead of adding it to every gap.
  let vaaAhead = fetchLatestVaa().catch(() => null);

  while (true) {
    const loopStart = Date.now();
    try {
      const guards = await findGuards(connection, config.wickProgramId);
      if (guards.length === 0) {
        console.log(
          "[wick-cranker] no guard accounts found on chain; check that the program is deployed and a guard is initialized"
        );
        await sleep(config.tickIntervalMs);
        continue;
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
        // The guard treats a gap of more than MAX_TICK_AGE_SLOTS (25, ~10s)
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

        console.log(
          `[wick-cranker] tick landed nonce=${nonce} guard=${pubkey.toBase58()} sig=${s3.slice(0, 8)}`
        );
      }
    } catch (err) {
      console.error(`[wick-cranker] tick failed: ${err.message}`);
      if (err.logs) console.error(err.logs.join("\n"));
    }
    // Sleep only the remainder of the interval. A fixed sleep would be added on
    // top of a tick that already took longer than the guard's staleness window,
    // guaranteeing the degrade it is meant to avoid.
    const spent = Date.now() - loopStart;
    if (spent < config.tickIntervalMs) await sleep(config.tickIntervalMs - spent);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
