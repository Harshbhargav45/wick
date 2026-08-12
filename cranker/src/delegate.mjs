/**
 * Move the cranker payer's guard PDA into the MagicBlock Ephemeral Rollup, and
 * back out again (§8.6).
 *
 *   node src/delegate.mjs status
 *   node src/delegate.mjs delegate [--validator <pubkey>]
 *   node src/delegate.mjs commit               # sync to base layer, stay delegated
 *   node src/delegate.mjs undelegate           # sync and hand ownership back
 *
 * `delegate` runs on the base layer; `commit`/`undelegate` must be sent to the
 * ER, because a delegated account is only writable there. The instruction
 * builders live in magicblock.mjs.
 */
import { Connection, PublicKey, Transaction } from "@solana/web3.js";
import { config, crankerKeypair } from "./config.mjs";
import {
  delegateIx,
  delegationPdas,
  describeOwner,
  magicIx,
  DELEGATION_PROGRAM_ID,
  IX_COMMIT,
  IX_COMMIT_AND_UNDELEGATE,
} from "./magicblock.mjs";
import { sharedConnection } from "./rpc.mjs";
import { guardPda } from "./tick.mjs";
import { G, GUARD_DATA_LEN } from "./guard-layout.mjs";

/**
 * A delegated guard is zeroed on the base layer, so `version` there reads 0 and
 * only the length is meaningful. The nonce is still worth printing when the
 * length matches — on the ER copy it is the live value.
 */
function describeNonce(data) {
  return data.length === GUARD_DATA_LEN
    ? String(data.readBigUInt64LE(G.nonce))
    : `? (len ${data.length}, expect ${GUARD_DATA_LEN})`;
}

async function send(connection, payer, ix, label) {
  const { blockhash, lastValidBlockHeight } =
    await connection.getLatestBlockhash("confirmed");
  const tx = new Transaction({
    feePayer: payer.publicKey,
    blockhash,
    lastValidBlockHeight,
  }).add(ix);
  const sig = await connection.sendTransaction(tx, [payer], {
    preflightCommitment: "confirmed",
  });
  await connection.confirmTransaction(
    { signature: sig, blockhash, lastValidBlockHeight },
    "confirmed"
  );
  console.log(`[${label}] -> ${sig}`);
  return sig;
}

async function status(base, guard) {
  const info = await base.getAccountInfo(guard);
  if (!info) {
    console.log(`[status] guard ${guard.toBase58()} does not exist`);
    return null;
  }
  console.log(`[status] guard    ${guard.toBase58()}`);
  console.log(`[status] owner    ${describeOwner(info.owner)}`);
  console.log(
    `[status] len      ${info.data.length} version=${info.data[G.version]} nonce=${describeNonce(info.data)}`
  );
  const { record, metadata, buffer } = delegationPdas(guard);
  for (const [name, pda] of [
    ["record  ", record],
    ["metadata", metadata],
    ["buffer  ", buffer],
  ]) {
    const acc = await base.getAccountInfo(pda);
    console.log(
      `[status] ${name} ${pda.toBase58()} ${acc ? `present len=${acc.data.length}` : "absent"}`
    );
  }
  // On the ER the delegated guard reads back as a live wick account.
  const er = new Connection(config.magicblockErUrl, "confirmed");
  try {
    const onEr = await er.getAccountInfo(guard);
    console.log(
      `[status] on ER    ${
        onEr
          ? `present len=${onEr.data.length} owner=${describeOwner(onEr.owner, "er")} nonce=${describeNonce(onEr.data)}`
          : "not present"
      }`
    );
  } catch (err) {
    console.log(`[status] on ER    query failed: ${err.message ?? err}`);
  }
  return info;
}

const cmd = process.argv[2] ?? "status";
const validatorArg = process.argv.indexOf("--validator");
const validator =
  validatorArg > -1 && process.argv[validatorArg + 1]
    ? new PublicKey(process.argv[validatorArg + 1])
    : undefined;

const base = sharedConnection();
const payer = crankerKeypair();
const { pda: guard, bump } = guardPda(payer.publicKey);

console.log(`[delegate] program ${config.wickProgramId.toBase58()}`);
console.log(`[delegate] owner   ${payer.publicKey.toBase58()}`);
console.log(`[delegate] base    ${base.__endpoints.join(" → ")}`);
console.log(`[delegate] er      ${config.magicblockErUrl}`);

if (cmd === "status") {
  await status(base, guard);
  process.exit(0);
}

const info = await status(base, guard);
if (!info) process.exit(1);

if (cmd === "delegate") {
  if (info.owner.equals(DELEGATION_PROGRAM_ID)) {
    console.log("\n[delegate] already delegated — nothing to do");
    process.exit(0);
  }
  if (validator) console.log(`[delegate] pinning validator ${validator.toBase58()}`);
  await send(
    base,
    payer,
    delegateIx({ payer: payer.publicKey, guard, bump, validator }),
    "delegate"
  );
  await status(base, guard);
} else if (cmd === "commit" || cmd === "undelegate") {
  if (!info.owner.equals(DELEGATION_PROGRAM_ID)) {
    console.log(`\n[${cmd}] guard is not delegated — run 'delegate' first`);
    process.exit(1);
  }
  // A delegated account is only writable on the ER, so these go there, not to
  // the base layer. No failover: the ER is a single endpoint by definition.
  const er = new Connection(config.magicblockErUrl, "confirmed");
  const discriminator = cmd === "commit" ? IX_COMMIT : IX_COMMIT_AND_UNDELEGATE;
  await send(er, payer, magicIx({ discriminator, payer: payer.publicKey, guard }), cmd);
  console.log(`[${cmd}] waiting for the base layer to catch up…`);
  await new Promise((r) => setTimeout(r, 15_000));
  await status(base, guard);
} else {
  console.log(`\nunknown command '${cmd}' — use status|delegate|commit|undelegate`);
  process.exit(1);
}
