/**
 * Tick a *delegated* guard on the MagicBlock Ephemeral Rollup (§8.6).
 *
 *   node src/tick-er.mjs [--once] [--ticks N]
 *
 * The split is forced by where each account lives:
 *
 *   base layer — Wormhole + the Pyth receiver, and the scratch accounts a
 *                `PriceUpdateV2` post creates. New accounts cannot be opened on
 *                the ER, so the price post has to happen here.
 *   ER         — the guard itself. Once delegated it is owned by the Delegation
 *                Program on the base layer, so a base-layer `OnPriceTick` fails
 *                the `guard.owned_by(program_id)` check. The ER hydrates it,
 *                reports wick as its owner, and accepts writes.
 *
 * So each tick posts the price on L1, then sends `OnPriceTick` to the ER, where
 * `route_config` and the price update are read-only clones pulled in on demand.
 * The scratch rent is reclaimed on L1 after the ER tick lands.
 */
import {
  ComputeBudgetProgram,
  Connection,
  PublicKey,
  Transaction,
} from "@solana/web3.js";
import { Buffer as IsomorphicBuffer } from "node:buffer";
import { config, cluster, crankerKeypair } from "./config.mjs";
import { fetchLatestVaa } from "./hermes.mjs";
import { buildPostUpdateInstructions } from "./receiver.mjs";
import { logEndpointConfig, sharedConnection } from "./rpc.mjs";
import { guardPda, routeConfigPda } from "./tick.mjs";
import { DELEGATION_PROGRAM_ID } from "./magicblock.mjs";
import {
  G,
  PENDING_IX_NAMES,
  RECONCILE_DIVERGED,
  SCALE,
  VENUE_DRIFT,
  readU128LE,
} from "./guard-layout.mjs";

const CLOCK_SYSVAR = new PublicKey(
  "SysvarC1ock11111111111111111111111111111111"
);

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
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

const base = sharedConnection();
const er = new Connection(config.magicblockErUrl, "confirmed");
const payer = crankerKeypair();
config.feePayerPublicKey = payer.publicKey;

const { pda: guard, bump } = guardPda(payer.publicKey);
const { pda: routeConfig } = routeConfigPda();

const once = process.argv.includes("--once");
const ticksArg = process.argv.indexOf("--ticks");
const maxTicks = once
  ? 1
  : ticksArg > -1
    ? Number(process.argv[ticksArg + 1])
    : Infinity;

console.log(`[er] program ${config.wickProgramId.toBase58()}`);
console.log(`[er] guard   ${guard.toBase58()}`);
logEndpointConfig(
  { cluster, endpoints: config.rpcEndpoints, rejected: config.rpcRejected },
  (m) => console.log(`[er] base    ${m.replace(/^\[rpc\] /, "")}`)
);
console.log(`[er] er      ${config.magicblockErUrl}`);

// A guard that is not delegated must be ticked on the base layer instead. Fail
// loudly rather than sending ER traffic that can only ever be rejected.
const baseInfo = await base.getAccountInfo(guard);
if (!baseInfo) {
  console.error(`[er] guard does not exist — run 'node src/init.mjs' first`);
  process.exit(1);
}
if (!baseInfo.owner.equals(DELEGATION_PROGRAM_ID)) {
  console.error(
    `[er] guard is not delegated (owner=${baseInfo.owner.toBase58()}) — run 'node src/delegate.mjs delegate'`
  );
  process.exit(1);
}

// §8.7 vs §8.6 — an autonomous Drift guard cannot execute here, and the
// operator has to know that *before* the breach rather than from a failed tick.
//
// The reduce is a CPI into Drift, which needs the venue's sub-account writable.
// The ER only permits writes to accounts delegated to it, and Drift's accounts
// are not; appending them would make every tick fail, including the healthy ones
// this loop exists to record. So the accounts are deliberately NOT appended, and
// the consequence is stated instead: health monitoring works on the ER, autonomous
// execution does not.
const erGuardPeek = await er.getAccountInfo(guard);
const venueByte = (erGuardPeek ?? baseInfo).data[G.venue];
if (venueByte === VENUE_DRIFT && (erGuardPeek ?? baseInfo).data[G.authorityReq] === 0) {
  console.error(
    "[er] WARNING: this is an autonomous Drift guard. Its reduce-only CPI needs Drift's\n" +
      "[er]          sub-account writable, and the ER can only write accounts delegated to it.\n" +
      "[er]          Ticks will land and health will be recorded, but a breach will FAIL here.\n" +
      "[er]          Undelegate before it matters: node src/delegate.mjs undelegate"
  );
}

let vaaAhead = fetchLatestVaa().catch(() => null);
let landed = 0;
let failed = 0;

for (let i = 0; i < maxTicks; i += 1) {
  const loopStart = Date.now();
  try {
    // The nonce lives on the ER copy now — the base-layer bytes are frozen at
    // whatever the last commit wrote, so reading them would replay a stale nonce
    // and trip the `expected_nonce` check.
    const erInfo = await er.getAccountInfo(guard);
    if (!erInfo) throw new Error("guard not present on the ER");
    const nonce = erInfo.data.readBigUInt64LE(G.nonce) + 1n;

    const fetched = (await vaaAhead) ?? (await fetchLatestVaa());
    vaaAhead = fetchLatestVaa().catch(() => null);
    const plans = await buildPostUpdateInstructions(fetched.vaa);

    const blockhash = (await base.getLatestBlockhash("confirmed")).blockhash;
    await sendAndConfirm(
      base,
      new Transaction().add(...plans.initTx),
      [payer, plans.encodedVaaSigner],
      blockhash
    );
    await sendAndConfirm(
      base,
      new Transaction().add(...plans.verifyTx),
      [payer],
      blockhash
    );
    // The price post stays on L1 and is confirmed before the tick, so the ER has
    // a finalized account to clone. Bundling it with the tick is not an option
    // here: they run on different layers.
    await sendAndConfirm(
      base,
      new Transaction().add(
        ComputeBudgetProgram.setComputeUnitLimit({ units: 250_000 }),
        plans.postUpdateIx
      ),
      [payer, plans.priceUpdateSigner],
      blockhash
    );

    const tickData = IsomorphicBuffer.alloc(10);
    tickData[0] = 7; // OnPriceTick
    tickData.writeBigUInt64LE(nonce, 1);
    tickData[9] = bump;

    const tickTx = new Transaction().add(
      ComputeBudgetProgram.setComputeUnitLimit({ units: 250_000 }),
      {
        programId: config.wickProgramId,
        keys: [
          { pubkey: guard, isSigner: false, isWritable: true },
          { pubkey: CLOCK_SYSVAR, isSigner: false, isWritable: false },
          { pubkey: routeConfig, isSigner: false, isWritable: false },
          {
            pubkey: plans.priceUpdateAccount,
            isSigner: false,
            isWritable: false,
          },
        ],
        data: tickData,
      }
    );
    const erStart = Date.now();
    const sig = await sendAndConfirm(er, tickTx, [payer]);
    const erMs = Date.now() - erStart;

    const after = await er.getAccountInfo(guard);
    landed += 1;
    console.log(
      `[er] tick landed nonce=${after.data.readBigUInt64LE(G.nonce)} ` +
        `price=${(Number(readU128LE(after.data, G.price)) / Number(SCALE)).toFixed(2)} ` +
        `ts=${after.data.readBigInt64LE(G.lastCheckTs)} ` +
        // `degraded` only flips on the third consecutive stale tick, so the
        // streak is the leading indicator — a run can look clean on `degraded`
        // alone while every tick is actually landing stale.
        `degraded=${after.data[G.degraded]} streak=${after.data[G.staleStreak]} ` +
        `${erMs}ms sig=${sig.slice(0, 8)}`
    );

    // A guard that has decided something is the whole point of the loop, so it
    // gets its own line rather than being inferable from an unchanged nonce.
    const pxKind = after.data[G.pendingIxKind];
    if (pxKind !== 0) {
      console.log(
        `[er] AWAITING OWNER: ${PENDING_IX_NAMES[pxKind] ?? `kind=${pxKind}`} built at expectedNonce=${after.data.readBigUInt64LE(G.pendingIxNonce)}`
      );
    }
    if (after.data[G.reconcileStatus] === RECONCILE_DIVERGED) {
      // Reconciliation itself runs on the base layer (index.mjs) — the venue's
      // accounts are not on the ER — but the verdict travels with the guard, and
      // it blocks autonomous execution, so it is reported wherever it is read.
      console.error(
        "[er] DIVERGED: the guard's model disagrees with the venue; autonomous execution is blocked until the owner runs UpdatePosition"
      );
    }

    // Best-effort scratch cleanup on L1, after the ER tick has read the account.
    try {
      const tx = new Transaction().add(plans.close, plans.reclaimRentIx);
      tx.feePayer = payer.publicKey;
      tx.recentBlockhash = (await base.getLatestBlockhash("confirmed")).blockhash;
      await base.sendTransaction(tx, [payer], { skipPreflight: true });
    } catch (e) {
      console.error(`[er] reclaim failed (best-effort): ${e.message}`);
    }
  } catch (err) {
    failed += 1;
    console.error(`[er] tick failed: ${err.message}`);
    if (err.logs) console.error(err.logs.join("\n"));
  }
  if (i + 1 < maxTicks) {
    const spent = Date.now() - loopStart;
    if (spent < config.tickIntervalMs) await sleep(config.tickIntervalMs - spent);
  }
}

console.log(`[er] done: ${landed} landed, ${failed} failed`);
