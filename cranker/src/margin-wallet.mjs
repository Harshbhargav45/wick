/**
 * Margin reserve CLI — Gap 3's cranker half (§8.5).
 *
 * A `TopUp` the owner cannot fund is a suggestion, not an action. The program
 * gained a real lamport reserve behind that action; this is how an operator
 * creates and funds one, and how they get value back out.
 *
 *   node src/margin-wallet.mjs init
 *   node src/margin-wallet.mjs fund   <SOL>
 *   node src/margin-wallet.mjs status
 *   node src/margin-wallet.mjs withdraw <SOL>   # needs the co-authority too
 *
 * Withdrawal is 2-of-2 by design, so it needs a second signer. The
 * co-authority keypair is read from `CO_AUTHORITY_KEYPAIR` — a path, never an
 * inline secret — and the command refuses rather than pretending when it is
 * absent.
 */
import { LAMPORTS_PER_SOL, PublicKey, Transaction } from "@solana/web3.js";
import { Buffer as IsomorphicBuffer } from "node:buffer";
import { readFileSync } from "node:fs";
import { Keypair } from "@solana/web3.js";
import { config, crankerKeypair } from "./config.mjs";
import { sharedConnection } from "./rpc.mjs";
import { guardPda, routeConfigPda } from "./tick.mjs";
import { G, IX, isGuardAccount } from "./guard-layout.mjs";

const RENT_SYSVAR = new PublicKey("SysvarRent111111111111111111111111111111111");
const SYSTEM_PROGRAM = new PublicKey("11111111111111111111111111111111");
const MARGIN_SEED = "margin";

/** `WalletState`, mirroring `program/src/account.rs`. */
const WALLET_DATA_LEN = 81;
const W = { version: 0, owner: 1, coAuthority: 33, balance: 65 };

export function marginWalletPda(venueOwner) {
  const [pda, bump] = PublicKey.findProgramAddressSync(
    [IsomorphicBuffer.from(MARGIN_SEED), venueOwner.toBuffer()],
    config.wickProgramId
  );
  return { pda, bump };
}

function readU128LE(d, off) {
  let v = 0n;
  for (let i = 15; i >= 0; i--) v = (v << 8n) | BigInt(d[off + i]);
  return v;
}

export function decodeWallet(data) {
  if (data?.length !== WALLET_DATA_LEN) {
    throw new Error(`not a margin wallet: len=${data?.length}`);
  }
  return {
    version: data[W.version],
    owner: new PublicKey(data.subarray(W.owner, W.owner + 32)),
    coAuthority: new PublicKey(data.subarray(W.coAuthority, W.coAuthority + 32)),
    balance: readU128LE(data, W.balance),
  };
}

/** `amount` is lamports, u128 LE — the program narrows it to u64 and rejects
 * anything that does not fit, so an out-of-range value fails there too. */
function amountData(disc, lamports) {
  const d = IsomorphicBuffer.alloc(17);
  d[0] = disc;
  let v = BigInt(lamports);
  for (let i = 0; i < 16; i++) {
    d[1 + i] = Number(v & 0xffn);
    v >>= 8n;
  }
  return d;
}

/**
 * SOL → lamports without floating-point drift. `2.5 * LAMPORTS_PER_SOL` is
 * exact, but `0.1 * 3 * LAMPORTS_PER_SOL` is not, and a reserve is value.
 */
export function solToLamports(sol) {
  const s = String(sol).trim();
  if (!/^\d+(\.\d+)?$/.test(s)) {
    throw new Error(`amount must be a positive decimal number of SOL, got "${sol}"`);
  }
  const [whole, frac = ""] = s.split(".");
  if (frac.length > 9) {
    throw new Error("SOL has 9 decimal places; that amount is not representable");
  }
  const lamports =
    BigInt(whole) * BigInt(LAMPORTS_PER_SOL) + BigInt(frac.padEnd(9, "0"));
  if (lamports === 0n) throw new Error("amount must be greater than zero");
  return lamports;
}

function loadCoAuthority() {
  const path = process.env.CO_AUTHORITY_KEYPAIR;
  if (!path) {
    throw new Error(
      "withdrawal is 2-of-2 (§8.5) and needs the co-authority's signature. " +
        "Set CO_AUTHORITY_KEYPAIR to its keypair file path."
    );
  }
  return Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(readFileSync(path, "utf8")))
  );
}

export function buildInitIx({ programId, wallet, bump, guard, owner, routeConfig }) {
  return {
    programId,
    keys: [
      { pubkey: wallet, isSigner: false, isWritable: true },
      { pubkey: guard, isSigner: false, isWritable: true },
      { pubkey: owner, isSigner: true, isWritable: false },
      { pubkey: owner, isSigner: true, isWritable: true }, // payer
      { pubkey: RENT_SYSVAR, isSigner: false, isWritable: false },
      { pubkey: routeConfig, isSigner: false, isWritable: false },
      { pubkey: SYSTEM_PROGRAM, isSigner: false, isWritable: false },
    ],
    data: IsomorphicBuffer.from([IX.InitMarginWallet, bump]),
  };
}

export function buildFundIx({ programId, wallet, guard, owner, routeConfig, lamports }) {
  return {
    programId,
    keys: [
      { pubkey: wallet, isSigner: false, isWritable: true },
      { pubkey: guard, isSigner: false, isWritable: false },
      { pubkey: owner, isSigner: true, isWritable: true },
      { pubkey: RENT_SYSVAR, isSigner: false, isWritable: false },
      { pubkey: routeConfig, isSigner: false, isWritable: false },
      { pubkey: SYSTEM_PROGRAM, isSigner: false, isWritable: false },
    ],
    data: amountData(IX.FundMarginWallet, lamports),
  };
}

export function buildWithdrawIx({
  programId,
  wallet,
  guard,
  owner,
  coAuthority,
  routeConfig,
  lamports,
}) {
  return {
    programId,
    keys: [
      { pubkey: wallet, isSigner: false, isWritable: true },
      { pubkey: guard, isSigner: false, isWritable: false },
      { pubkey: owner, isSigner: true, isWritable: true },
      { pubkey: coAuthority, isSigner: true, isWritable: false },
      { pubkey: RENT_SYSVAR, isSigner: false, isWritable: false },
      { pubkey: routeConfig, isSigner: false, isWritable: false },
    ],
    data: amountData(IX.WithdrawMarginWallet, lamports),
  };
}

async function main() {
  const [command, arg] = process.argv.slice(2);
  const connection = sharedConnection();
  const payer = crankerKeypair();
  const owner = payer.publicKey;
  const { pda: guard } = guardPda(owner);
  const { pda: routeConfig } = routeConfigPda();
  const { pda: wallet, bump } = marginWalletPda(owner);
  const programId = config.wickProgramId;

  const send = async (ix, signers) => {
    const tx = new Transaction().add(ix);
    tx.feePayer = owner;
    tx.recentBlockhash = (await connection.getLatestBlockhash("confirmed")).blockhash;
    if (config.dryRun) {
      console.log("[margin] DRY_RUN=1 — built but not sent. Set DRY_RUN=0 to send.");
      return null;
    }
    const sig = await connection.sendTransaction(tx, signers, {
      skipPreflight: false,
      preflightCommitment: "confirmed",
    });
    await connection.confirmTransaction(sig, "confirmed");
    return sig;
  };

  console.log(`program  ${programId.toBase58()}`);
  console.log(`owner    ${owner.toBase58()}`);
  console.log(`guard    ${guard.toBase58()}`);
  console.log(`wallet   ${wallet.toBase58()} (bump ${bump})`);

  const guardInfo = await connection.getAccountInfo(guard);
  if (!guardInfo) {
    console.error("[margin] no guard at that PDA — run `npm run init` first");
    process.exit(1);
  }
  if (!isGuardAccount(guardInfo.data)) {
    console.error(
      `[margin] guard is not a current-version account (len=${guardInfo.data.length} version=${guardInfo.data[0]}); redeploy or migrate before using the reserve`
    );
    process.exit(1);
  }

  switch (command) {
    case "status": {
      const linkedBump = guardInfo.data[G.marginWalletBump];
      console.log(
        `linked   ${linkedBump === 0 ? "NO — an autonomous TopUp will escalate instead of executing" : `yes (bump ${linkedBump})`}`
      );
      const info = await connection.getAccountInfo(wallet);
      if (!info) {
        console.log("reserve  does not exist — run `node src/margin-wallet.mjs init`");
        break;
      }
      const ws = decodeWallet(info.data);
      const rentMin = await connection.getMinimumBalanceForRentExemption(
        WALLET_DATA_LEN
      );
      console.log(`balance  ${Number(ws.balance) / LAMPORTS_PER_SOL} SOL (credited)`);
      console.log(
        `lamports ${info.lamports} (rent floor ${rentMin}; backing ${info.lamports - rentMin})`
      );
      console.log(`coAuth   ${ws.coAuthority.toBase58()}`);
      // The invariant the program enforces on every mutation. Printing it means
      // a violation is visible here rather than only as a failed transaction.
      if (BigInt(info.lamports - rentMin) < ws.balance) {
        console.error(
          "[margin] !! reserve claims more than its lamports back — the invariant is violated"
        );
        process.exitCode = 1;
      }
      break;
    }
    case "init": {
      const sig = await send(
        buildInitIx({ programId, wallet, bump, guard, owner, routeConfig }),
        [payer]
      );
      console.log(sig ? `[margin] reserve created sig=${sig}` : "[margin] built init");
      break;
    }
    case "fund": {
      const lamports = solToLamports(arg ?? "");
      const sig = await send(
        buildFundIx({ programId, wallet, guard, owner, routeConfig, lamports }),
        [payer]
      );
      console.log(
        sig
          ? `[margin] funded ${arg} SOL (${lamports} lamports) sig=${sig}`
          : `[margin] built fund for ${lamports} lamports`
      );
      break;
    }
    case "withdraw": {
      const lamports = solToLamports(arg ?? "");
      const co = loadCoAuthority();
      const ws = decodeWallet(
        (await connection.getAccountInfo(wallet))?.data ?? IsomorphicBuffer.alloc(0)
      );
      if (!co.publicKey.equals(ws.coAuthority)) {
        console.error(
          `[margin] CO_AUTHORITY_KEYPAIR is ${co.publicKey.toBase58()} but the reserve's co-authority is ${ws.coAuthority.toBase58()}`
        );
        process.exit(1);
      }
      const sig = await send(
        buildWithdrawIx({
          programId,
          wallet,
          guard,
          owner,
          coAuthority: co.publicKey,
          routeConfig,
          lamports,
        }),
        [payer, co]
      );
      console.log(
        sig
          ? `[margin] withdrew ${arg} SOL sig=${sig}`
          : `[margin] built withdraw for ${lamports} lamports`
      );
      break;
    }
    default:
      console.error(
        "usage: node src/margin-wallet.mjs <status|init|fund SOL|withdraw SOL>"
      );
      process.exit(1);
  }
}

// Only run as a CLI — the builders above are imported by the tests.
if (process.argv[1] && process.argv[1].endsWith("margin-wallet.mjs")) {
  main().catch((err) => {
    console.error(`[margin] ${err.message}`);
    process.exit(1);
  });
}
