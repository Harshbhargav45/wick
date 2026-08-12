/**
 * Read-only inspector: prints the on-chain state of the RouteConfig, the
 * cranker payer's guard PDA, its margin reserve, and every guard the program
 * owns.
 *
 *   node src/inspect.mjs
 *
 * The offsets come from `guard-layout.mjs` (which is tested against
 * `program/src/account.rs`) rather than being re-copied here — a private copy is
 * how this file previously came to print v2 offsets for a v3 account, which
 * reports plausible-looking wrong numbers instead of failing.
 */
import { PublicKey } from "@solana/web3.js";
import { config, crankerKeypair } from "./config.mjs";
import { sharedConnection } from "./rpc.mjs";
import { guardPda, routeConfigPda } from "./tick.mjs";
import { decodeWallet, marginWalletPda } from "./margin-wallet.mjs";
import {
  ACCOUNT_VERSION,
  G,
  GUARD_DATA_LEN,
  PENDING_IX_NAMES,
  RECONCILE_DIVERGED,
  RECONCILE_NAMES,
  RECONCILE_NEVER,
  ROUTE_CONFIG_LEN,
  SCALE,
  U128_MAX,
  VENUE_DRIFT,
  VENUE_NAMES,
  readI128LE,
  readU128LE,
} from "./guard-layout.mjs";

const DELEGATION_PROGRAM_ID = new PublicKey(
  "DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh"
);

function usd(v) {
  if (v === U128_MAX) return "unset";
  return (Number(v) / Number(SCALE)).toFixed(2);
}

/**
 * `last_check_ts` is wall-clock seconds (§8.6). A guard written before the
 * slot→seconds change holds a slot number here, which as a timestamp lands in
 * the 1980s — worth showing plainly, since that is what costs the first
 * post-upgrade tick its freshness.
 */
function describeTs(ts) {
  if (ts <= 0n) return `${ts} (never)`;
  return `${ts} (${new Date(Number(ts) * 1000).toISOString()})`;
}

function describeOwner(owner) {
  if (owner.equals(config.wickProgramId)) return "wick (on base layer)";
  if (owner.equals(DELEGATION_PROGRAM_ID)) return "DELEGATED to MagicBlock ER";
  return owner.toBase58();
}

function printGuard(pubkey, info) {
  const d = info.data;
  console.log(`\nguard ${pubkey.toBase58()}`);
  console.log(`  owner        ${describeOwner(info.owner)}`);
  console.log(`  len          ${d.length} (expect ${GUARD_DATA_LEN})`);
  if (d.length !== GUARD_DATA_LEN) {
    console.log("  !! unexpected length — not decoding further");
    return;
  }
  if (d.every((b) => b === 0)) {
    // delegate_account zeroes the PDA before assigning it away, and CloseGuard
    // zeroes it on purpose. Either way there is nothing to decode.
    console.log("  !! all zero — delegated away, or closed");
    return;
  }
  if (d[G.version] !== ACCOUNT_VERSION) {
    console.log(
      `  !! version ${d[G.version]}, expected ${ACCOUNT_VERSION} — the offsets below are for v${ACCOUNT_VERSION} and would be wrong`
    );
    return;
  }
  const venue = d[G.venue];
  console.log(`  version      ${d[G.version]}`);
  console.log(`  venue        ${venue} (${VENUE_NAMES[venue] ?? "?"})`);
  console.log(
    `  authority    ${d[G.authorityReq] === 0 ? "Autonomous" : "CoSigned"}`
  );
  console.log(
    `  venueOwner   ${new PublicKey(d.subarray(G.venueOwner, G.venueOwner + 32)).toBase58()}`
  );
  console.log(
    `  coAuthority  ${new PublicKey(d.subarray(G.coAuthority, G.coAuthority + 32)).toBase58()}`
  );
  console.log(`  collateral   ${usd(readU128LE(d, G.collateral))}`);
  console.log(`  size         ${usd(readI128LE(d, G.size))}`);
  console.log(`  entry        ${usd(readU128LE(d, G.entry))}`);
  console.log(`  price        ${usd(readU128LE(d, G.price))}`);
  console.log(`  takeProfit   ${usd(readU128LE(d, G.takeProfit))}`);
  console.log(`  nonce        ${d.readBigUInt64LE(G.nonce)}`);
  console.log(`  lastCheckTs  ${describeTs(d.readBigInt64LE(G.lastCheckTs))}`);
  console.log(
    `  pending      ${d[G.pendingTag] === 0 ? "none" : `tag=${d[G.pendingTag]} amount=${usd(readU128LE(d, G.pendingAmount))}`}`
  );

  // §8.4 — a co-signed venue's action exists as a pre-built instruction waiting
  // for the owner's signature. If one is sitting here, the guard has already
  // decided and is blocked on a human.
  const pxKind = d[G.pendingIxKind];
  console.log(
    `  pendingIx    ${
      pxKind === 0
        ? "none"
        : `${PENDING_IX_NAMES[pxKind] ?? `kind=${pxKind}`} expectedNonce=${d.readBigUInt64LE(G.pendingIxNonce)} — AWAITING OWNER SIGNATURE`
    }`
  );

  console.log(
    `  degraded     ${d[G.degraded]}  staleStreak=${d[G.staleStreak]}`
  );

  // §8.5 — the daily budget. An exhausted budget turns every subsequent breach
  // into an escalation, which looks like the guard has stopped working.
  console.log(
    `  dailySpent   ${usd(readU128LE(d, G.dailySpentUsd))} (epoch started ${describeTs(d.readBigInt64LE(G.dailyEpochStartTs))})`
  );

  // §8.3 — the reconcile verdict. DIVERGED blocks autonomous execution, and it
  // is the single most important line here when it is set.
  const status = d[G.reconcileStatus];
  const line = `  reconcile    ${RECONCILE_NAMES[status] ?? `status=${status}`} nonce=${d.readBigUInt64LE(G.reconcileNonce)} at ${describeTs(d.readBigInt64LE(G.reconcileTs))}`;
  if (status === RECONCILE_DIVERGED) {
    console.log(line);
    console.log(
      `  !! DIVERGED  model size=${usd(readI128LE(d, G.size))} collateral=${usd(readU128LE(d, G.collateral))}`
    );
    console.log(
      `  !!           venue size=${usd(readI128LE(d, G.venueSize))} collateral=${usd(readU128LE(d, G.venueCollateral))}`
    );
    console.log(
      "  !!           autonomous execution is blocked until the owner runs UpdatePosition"
    );
  } else {
    console.log(line);
    if (status === RECONCILE_NEVER && venue === VENUE_DRIFT) {
      console.log(
        "  !!           never reconciled — the guard has not yet checked its model against the venue"
      );
    }
  }

  // §8.5 — an autonomous TopUp draws on this. Bump 0 means no reserve is
  // linked, so a top-up escalates rather than executing.
  const walletBump = d[G.marginWalletBump];
  console.log(
    `  marginWallet ${walletBump === 0 ? "NOT LINKED — an autonomous TopUp will escalate" : `bump=${walletBump}`}`
  );

  console.log(
    `  drift        market=${d.readUInt16LE(G.driftMarket)} subaccount=${d.readUInt16LE(G.driftSubaccount)}`
  );
}

const connection = sharedConnection();
const payer = crankerKeypair();

console.log(`program  ${config.wickProgramId.toBase58()}`);
console.log(`rpc      ${connection.__endpoints.join(" → ")}`);

const programInfo = await connection.getAccountInfo(config.wickProgramId);
console.log(`deployed ${!!programInfo?.executable}`);

const { pda: routeConfig } = routeConfigPda();
const routeInfo = await connection.getAccountInfo(routeConfig);
if (routeInfo?.data.length === ROUTE_CONFIG_LEN) {
  console.log(
    `\nrouteConfig ${routeConfig.toBase58()}\n  version    ${routeInfo.data[0]}\n  authority  ${new PublicKey(routeInfo.data.subarray(1, 33)).toBase58()}\n  paused     ${routeInfo.data[33] === 1}`
  );
  if (routeInfo.data[33] === 1) {
    console.log(
      "  !! PAUSED  every state-mutating instruction is refused, including ticks"
    );
  }
} else if (routeInfo) {
  console.log(
    `\nrouteConfig ${routeConfig.toBase58()} unexpected length ${routeInfo.data.length} (expect ${ROUTE_CONFIG_LEN})`
  );
} else {
  console.log(`\nrouteConfig ${routeConfig.toBase58()} MISSING`);
}

const { pda: mine, bump } = guardPda(payer.publicKey);
console.log(`\ncranker payer ${payer.publicKey.toBase58()}`);
console.log(
  `  balance     ${((await connection.getBalance(payer.publicKey)) / 1e9).toFixed(4)} SOL`
);
console.log(`  its guard   ${mine.toBase58()} bump=${bump}`);
const mineInfo = await connection.getAccountInfo(mine);
if (mineInfo) printGuard(mine, mineInfo);
else console.log("  its guard   NOT CREATED");

// The reserve behind an autonomous TopUp. Reported separately because it is a
// separate account: the guard can say "linked" while the reserve holds nothing.
const { pda: walletPda } = marginWalletPda(payer.publicKey);
const walletInfo = await connection.getAccountInfo(walletPda);
console.log(`\nmarginWallet ${walletPda.toBase58()}`);
if (!walletInfo) {
  console.log("  not created — `node src/margin-wallet.mjs init`");
} else {
  try {
    const ws = decodeWallet(walletInfo.data);
    const rentMin = await connection.getMinimumBalanceForRentExemption(
      walletInfo.data.length
    );
    console.log(`  credited    ${Number(ws.balance) / 1e9} SOL`);
    console.log(
      `  lamports    ${walletInfo.lamports} (rent floor ${rentMin}, backing ${walletInfo.lamports - rentMin})`
    );
    console.log(`  coAuthority ${ws.coAuthority.toBase58()}`);
    if (ws.balance === 0n) {
      console.log(
        "  !!          empty — an autonomous TopUp has nothing to draw on and will escalate"
      );
    }
    if (BigInt(walletInfo.lamports - rentMin) < ws.balance) {
      console.log(
        "  !!          claims more than its lamports back — the reserve invariant is violated"
      );
    }
  } catch (err) {
    console.log(`  !! ${err.message}`);
  }
}

// A delegated guard is owned by the Delegation Program, so filtering by owner
// would hide exactly the accounts this script exists to show.
const owned = await connection.getProgramAccounts(config.wickProgramId, {
  filters: [{ dataSize: GUARD_DATA_LEN }],
});
console.log(`\nwick-owned guards: ${owned.length}`);
for (const a of owned) {
  if (!a.pubkey.equals(mine)) printGuard(a.pubkey, a.account);
}
