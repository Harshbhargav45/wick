/**
 * One-shot on-chain initialization for a fresh deployment.
 *
 *   node src/init.mjs [--venue drift|jupiter|none] [--collateral N] [--size N] [--entry N]
 *
 * Creates the singleton RouteConfig (kill-switch), creates the guard PDA for
 * the cranker keypair, and enrolls a starting position so the guard has real
 * state to run health against. Each step is skipped if it already exists, so
 * re-running is safe.
 *
 * Layouts mirror `init_route_config`, `init_guard` and `update_position` in
 * program/src/processor.rs.
 */

import {
  PublicKey,
  SystemProgram,
  Transaction,
} from "@solana/web3.js";
import { Buffer as IsomorphicBuffer } from "node:buffer";
import { config, crankerKeypair } from "./config.mjs";
import { sharedConnection } from "./rpc.mjs";
import { guardPda, routeConfigPda } from "./tick.mjs";
import {
  ACCOUNT_VERSION,
  GUARD_DATA_LEN,
  ROUTE_CONFIG_LEN,
  SCALE,
  U128_MAX,
  VENUE_DRIFT,
  VENUE_JUPITER,
  VENUE_NONE,
  writeU128LE,
} from "./guard-layout.mjs";

const IX_INIT_GUARD = 0;
const IX_UPDATE_POSITION = 8;
const IX_INIT_ROUTE_CONFIG = 10;

const RENT_SYSVAR = new PublicKey("SysvarRent111111111111111111111111111111111");

/**
 * Account creation happens inside the handler via a `CreateAccount` CPI, so the
 * system program has to be in the transaction for the runtime to resolve it.
 * `split_4` only requires a minimum length, so trailing accounts are fine.
 */
function createAccountKeys(target, payer) {
  return [
    { pubkey: target, isSigner: false, isWritable: true },
    { pubkey: payer, isSigner: true, isWritable: false },
    { pubkey: payer, isSigner: true, isWritable: true },
    { pubkey: RENT_SYSVAR, isSigner: false, isWritable: false },
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ];
}

const VENUES = { none: VENUE_NONE, jupiter: VENUE_JUPITER, drift: VENUE_DRIFT };

/** Autonomous = guard signs its own CPI; CoSigned = guard only builds. */
const AUTHORITY = { autonomous: 0, cosigned: 1 };

function parseArgs(argv) {
  const out = {};
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith("--")) out[a.slice(2)] = argv[++i] ?? "true";
  }
  return out;
}

/**
 * InitGuard payload after [discriminator, bump]: a 150-byte policy blob.
 * Offsets are the ones `parse_policy` reads.
 */
function buildInitGuardData(bump, opts) {
  const data = IsomorphicBuffer.alloc(2 + 150);
  data[0] = IX_INIT_GUARD;
  data[1] = bump;

  const p = 2;
  data[p + 0] = opts.venue;
  opts.coAuthority.toBuffer().copy(data, p + 1);
  data[p + 33] = opts.authorityReq;

  writeU128LE(data, opts.maintenanceBps, p + 34);
  writeU128LE(data, opts.triggerBufferBps, p + 50);
  writeU128LE(data, opts.feeBps, p + 66);
  writeU128LE(data, opts.capTopUp, p + 82);
  writeU128LE(data, opts.capPartialClose, p + 98);
  writeU128LE(data, opts.capDaily, p + 114);
  writeU128LE(data, opts.takeProfit, p + 130);
  data.writeUInt16LE(opts.driftMarketIndex, p + 146);
  data.writeUInt16LE(opts.driftSubaccountId, p + 148);
  return data;
}

function buildUpdatePositionData({ collateral, size, entry }) {
  const data = IsomorphicBuffer.alloc(49);
  data[0] = IX_UPDATE_POSITION;
  writeU128LE(data, collateral, 1);
  writeU128LE(data, size, 17);
  writeU128LE(data, entry, 33);
  return data;
}

async function sendAndConfirm(connection, payer, ix, label) {
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
  console.log(`[init] ${label} -> ${sig}`);
  return sig;
}

/** Initialized means program-owned, right length, and version byte stamped. */
function isInitialized(info, expectedLen) {
  return (
    info !== null &&
    info.data.length === expectedLen &&
    info.data[0] === ACCOUNT_VERSION
  );
}

async function main() {
  const args = parseArgs(process.argv);
  const payer = crankerKeypair();
  const connection = sharedConnection();
  const programId = config.wickProgramId;

  console.log(`[init] rpc=${connection.__endpoints.join(" → ")}`);
  console.log(`[init] program=${programId.toBase58()}`);
  console.log(`[init] owner/payer=${payer.publicKey.toBase58()}`);

  const programInfo = await connection.getAccountInfo(programId);
  if (!programInfo?.executable) {
    throw new Error(
      `program ${programId.toBase58()} is not deployed on this cluster`
    );
  }

  const balance = await connection.getBalance(payer.publicKey);
  console.log(`[init] balance=${(balance / 1e9).toFixed(4)} SOL`);
  if (balance === 0) {
    throw new Error("payer has no SOL — airdrop first: solana airdrop 2 --url devnet");
  }

  // ---- 1. RouteConfig (singleton kill-switch) -----------------------------
  // Every state-mutating instruction calls check_not_paused, which fails when
  // this account is missing. It has to exist before anything else works.
  const { pda: routeConfig, bump: routeBump } = routeConfigPda();
  const routeInfo = await connection.getAccountInfo(routeConfig);

  if (isInitialized(routeInfo, ROUTE_CONFIG_LEN)) {
    const authority = new PublicKey(routeInfo.data.subarray(1, 33));
    console.log(
      `[init] route config exists ${routeConfig.toBase58()} authority=${authority.toBase58()} paused=${routeInfo.data[33] === 1}`
    );
  } else {
    const data = IsomorphicBuffer.alloc(2);
    data[0] = IX_INIT_ROUTE_CONFIG;
    data[1] = routeBump;
    await sendAndConfirm(
      connection,
      payer,
      {
        programId,
        keys: createAccountKeys(routeConfig, payer.publicKey),
        data,
      },
      `route config ${routeConfig.toBase58()}`
    );
  }

  // ---- 2. Guard PDA -------------------------------------------------------
  const { pda: guard, bump: guardBump } = guardPda(payer.publicKey);
  const guardInfo = await connection.getAccountInfo(guard);

  // Default to the venue-less co-signed tier: it needs no venue accounts and no
  // funded reserve, so a bring-up produces a guard that ticks on the first try.
  // `--venue drift` is the autonomous tier — the tick loop now assembles the
  // adapter's accounts itself (see venue-drift.mjs), but an autonomous TopUp
  // still needs a funded margin reserve or it escalates instead of executing.
  const venueName = (args.venue ?? "none").toLowerCase();
  if (!(venueName in VENUES)) {
    throw new Error(`unknown venue "${venueName}" (none|jupiter|drift)`);
  }
  const venue = VENUES[venueName];

  // Drift is the autonomous tier — the guard PDA is the position delegate and
  // signs its own reduce-only CPI. Jupiter always requires the owner (§8.4).
  const authorityReq =
    venue === VENUES.drift ? AUTHORITY.autonomous : AUTHORITY.cosigned;

  if (isInitialized(guardInfo, GUARD_DATA_LEN)) {
    console.log(`[init] guard exists ${guard.toBase58()}`);
  } else {
    const data = buildInitGuardData(guardBump, {
      venue,
      // 2-of-2 withdrawal needs a second key. Self-co-authority is only
      // acceptable because this is a devnet bring-up.
      coAuthority: payer.publicKey,
      authorityReq,
      maintenanceBps: 500n, // 5% maintenance margin
      triggerBufferBps: 200n, // act at 7%, 200bps above maintenance
      feeBps: 10n,
      capTopUp: 1_000n * SCALE,
      capPartialClose: 5_000n * SCALE,
      capDaily: 20_000n * SCALE,
      // TakeProfit fires on price crossing alone, so `--take-profit` just below
      // spot is the one policy that makes a tick produce a visible action
      // without waiting for a real breach. Unset means the U128_MAX sentinel.
      takeProfit: args["take-profit"]
        ? BigInt(args["take-profit"]) * SCALE
        : U128_MAX,
      driftMarketIndex: Number(args["market-index"] ?? 0),
      driftSubaccountId: Number(args["subaccount-id"] ?? 0),
    });

    await sendAndConfirm(
      connection,
      payer,
      {
        programId,
        keys: createAccountKeys(guard, payer.publicKey),
        data,
      },
      `guard ${guard.toBase58()} venue=${venueName} authority=${authorityReq === 0 ? "autonomous" : "cosigned"}`
    );
  }

  // ---- 3. Position snapshot ----------------------------------------------
  // A guard with size 0 has nothing to protect and every tick is a no-op, so
  // enroll a position to make the health engine produce a real reading.
  const collateral = BigInt(args.collateral ?? 1_000) * SCALE;
  const size = BigInt(args.size ?? 10) * SCALE;
  const entry = BigInt(args.entry ?? 150) * SCALE;

  await sendAndConfirm(
    connection,
    payer,
    {
      programId,
      keys: [
        { pubkey: guard, isSigner: false, isWritable: true },
        { pubkey: payer.publicKey, isSigner: true, isWritable: false },
        { pubkey: routeConfig, isSigner: false, isWritable: false },
      ],
      data: buildUpdatePositionData({ collateral, size, entry }),
    },
    `position collateral=${collateral / SCALE} size=${size / SCALE} entry=${entry / SCALE}`
  );

  console.log("");
  console.log("[init] done. Put this in frontend/.env.local:");
  console.log(`  NEXT_PUBLIC_GUARD_PROGRAM_ID=${programId.toBase58()}`);
  console.log(`  NEXT_PUBLIC_SOLANA_RPC=${config.rpc}`);
  console.log(`[init] guard PDA: ${guard.toBase58()}`);
  // Said here rather than silently creating one: an *unfunded* reserve behaves
  // exactly like no reserve — the top-up escalates either way — so creating the
  // account without moving value in would only look like the gap was closed.
  if (authorityReq === AUTHORITY.autonomous) {
    console.log(
      "[init] this guard is autonomous. An autonomous TopUp draws on a margin reserve;\n" +
        "       until one is funded, a top-up escalates for manual review instead of executing:\n" +
        "         node src/margin-wallet.mjs init && node src/margin-wallet.mjs fund 1"
    );
  }
}

main().catch((err) => {
  console.error(`[init] failed: ${err.message}`);
  if (err.logs) console.error(err.logs.join("\n"));
  process.exit(1);
});
