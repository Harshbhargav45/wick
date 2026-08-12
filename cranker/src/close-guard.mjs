/**
 * Close a guard PDA (owner only): refund its rent and zero it so the owner can
 * re-init at the same address.
 *
 *   node src/close-guard.mjs [--yes]
 *
 * The guard PDA is a pure function of `b"guard" || owner`, so an account that
 * no longer decodes — a v1 guard under a v2 program, say — is a permanent
 * tombstone at the only address that owner will ever get, with its rent stuck.
 * `CloseGuard` (discriminator 11) is the way out. Mirrors `close_guard` in
 * program/src/processor.rs.
 *
 * This destroys guard state. It refuses to run without `--yes`.
 */
import { Transaction } from "@solana/web3.js";
import { Buffer as IsomorphicBuffer } from "node:buffer";
import { config, crankerKeypair } from "./config.mjs";
import { sharedConnection } from "./rpc.mjs";
import { guardPda } from "./tick.mjs";

const IX_CLOSE_GUARD = 11;

const connection = sharedConnection();
const payer = crankerKeypair();
const { pda: guard, bump } = guardPda(payer.publicKey);

const info = await connection.getAccountInfo(guard);
if (!info) {
  console.log(`[close] ${guard.toBase58()} does not exist — nothing to do`);
  process.exit(0);
}

console.log(`[close] owner  ${payer.publicKey.toBase58()}`);
console.log(`[close] guard  ${guard.toBase58()} bump=${bump}`);
console.log(`[close] len    ${info.data.length} version=${info.data[0]}`);
console.log(`[close] rent   ${(info.lamports / 1e9).toFixed(6)} SOL → refunded to owner`);

if (!process.argv.includes("--yes")) {
  console.log("\n[close] refusing without --yes (this erases guard state)");
  process.exit(1);
}

const data = IsomorphicBuffer.alloc(2);
data[0] = IX_CLOSE_GUARD;
data[1] = bump;

const { blockhash, lastValidBlockHeight } =
  await connection.getLatestBlockhash("confirmed");
const tx = new Transaction({
  feePayer: payer.publicKey,
  blockhash,
  lastValidBlockHeight,
}).add({
  programId: config.wickProgramId,
  keys: [
    { pubkey: guard, isSigner: false, isWritable: true },
    { pubkey: payer.publicKey, isSigner: true, isWritable: true },
  ],
  data,
});

const sig = await connection.sendTransaction(tx, [payer], {
  preflightCommitment: "confirmed",
});
await connection.confirmTransaction(
  { signature: sig, blockhash, lastValidBlockHeight },
  "confirmed"
);
console.log(`[close] closed -> ${sig}`);

const after = await connection.getAccountInfo(guard);
console.log(
  `[close] after: ${after ? `still present len=${after.data.length} lamports=${after.lamports}` : "gone (PDA free to re-init)"}`
);
