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

async function sendAndConfirm(connection, tx, signers) {
  const blockhash = (await connection.getLatestBlockhash("confirmed")).blockhash;
  tx.feePayer = config.feePayerPublicKey;
  tx.recentBlockhash = blockhash;
  const sig = await connection.sendTransaction(tx, signers, {
    skipPreflight: false,
    preflightCommitment: "confirmed",
  });
  await connection.confirmTransaction(sig, "confirmed");
  return sig;
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

  while (true) {
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

        const { vaa } = await fetchLatestVaa();
        const plans = await buildPostUpdateInstructions(vaa);

        // 1) post the encoded VAA to the Wormhole program
        // 2) post the price update to the Pyth receiver (Full verification)
        // 3) drive OnPriceTick on the guard
        const initTx = new Transaction().add(...plans.initTx);
        const verifyTx = new Transaction().add(...plans.verifyTx);
        const postTx = new Transaction().add(
          ComputeBudgetProgram.setComputeUnitLimit({ units: 100_000 }),
          plans.postUpdateIx
        );

        const tickData = IsomorphicBuffer.alloc(9);
        tickData[0] = 7; // OnPriceTick discriminator
        tickData.writeBigUInt64LE(nonce, 1);
        tickData[8] = bump;

        const tickTx = new Transaction().add({
          programId: config.wickProgramId,
          keys: [
            { pubkey, isSigner: false, isWritable: true },
            { pubkey: CLOCK_SYSVAR, isSigner: false, isWritable: false },
            { pubkey: routeConfig, isSigner: false, isWritable: false },
            { pubkey: plans.priceUpdateAccount, isSigner: false, isWritable: false },
          ],
          data: tickData,
        });

        if (config.dryRun) {
          console.log(
            `[wick-cranker] dry-run: guard=${pubkey.toBase58()} priceUpdate=${plans.priceUpdateAccount.toBase58()} nonce=${nonce}`
          );
          continue;
        }

        const s1 = await sendAndConfirm(connection, initTx, [
          payer,
          plans.encodedVaaSigner,
        ]);
        const s2 = await sendAndConfirm(connection, verifyTx, [
          payer,
          plans.encodedVaaSigner,
        ]);
        const s3 = await sendAndConfirm(connection, postTx, [
          payer,
          plans.priceUpdateSigner,
        ]);
        const s4 = await sendAndConfirm(connection, tickTx, [payer]);

        console.log(
          `[wick-cranker] tick sent nonce=${nonce} guard=${pubkey.toBase58()} post=${s3.slice(0, 8)} tick=${s4.slice(0, 8)}`
        );
      }
    } catch (err) {
      console.error(`[wick-cranker] tick failed: ${err.message}`);
    }
    await sleep(config.tickIntervalMs);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
