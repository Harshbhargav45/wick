import {
  ComputeBudgetProgram,
  Transaction,
  VersionedTransaction,
} from "@solana/web3.js";
import { config, crankerKeypair } from "./config.mjs";
import { fetchLatestVaa } from "./hermes.mjs";
import { buildPostUpdateInstructions } from "./receiver.mjs";
import { sharedConnection } from "./rpc.mjs";

const payer = crankerKeypair();
const connection = sharedConnection();

async function send(tx, signers) {
  const blockhash = (await connection.getLatestBlockhash("confirmed")).blockhash;
  tx.feePayer = payer.publicKey;
  tx.recentBlockhash = blockhash;
  const sig = await connection.sendTransaction(tx, signers, {
    skipPreflight: false,
    preflightCommitment: "confirmed",
  });
  await connection.confirmTransaction(sig, "confirmed");
  return sig;
}

const { vaa } = await fetchLatestVaa();
console.log(`[post-vaa] fetched VAA len=${vaa.length}`);

const plans = await buildPostUpdateInstructions(vaa);
console.log(`[post-vaa] initTx=${plans.initTx.length} verifyTx=${plans.verifyTx.length}`);

const sig1 = await send(
  new Transaction().add(...plans.initTx),
  [payer, plans.encodedVaaSigner]
);
console.log(`[post-vaa] init+write tx1: ${sig1}`);

const sig2 = await send(
  new Transaction().add(...plans.verifyTx),
  [payer, plans.encodedVaaSigner]
);
console.log(`[post-vaa] verify tx2: ${sig2}`);

const sig3 = await send(
  new Transaction().add(
    ComputeBudgetProgram.setComputeUnitLimit({ units: 100_000 }),
    plans.postUpdateIx
  ),
  [payer, plans.priceUpdateSigner]
);
console.log(`[post-vaa] postUpdate tx3: ${sig3}`);
console.log(`[post-vaa] PriceUpdateV2 account: ${plans.priceUpdateAccount.toBase58()}`);

const info = await connection.getAccountInfo(plans.priceUpdateAccount);
console.log(`[post-vaa] account exists: ${!!info} owner=${info?.owner.toBase58() ?? "n/a"}`);
