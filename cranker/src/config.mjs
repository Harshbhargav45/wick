import "dotenv/config";
import { PublicKey } from "@solana/web3.js";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { Keypair } from "@solana/web3.js";
import { resolveEndpointConfig } from "./rpc.mjs";

// One cluster per process. The guard, its PDA, and the Pyth feeds all live on
// the same chain, so the whole cranker is bound to one cluster at startup.
const rpcConfig = resolveEndpointConfig();
export const cluster = rpcConfig.cluster;

export const config = {
  // A prioritized list, not one URL — see rpc.mjs. `rpc` stays as the head of
  // that list for the places that just need a string (logging, explorer links).
  // `rejected` carries what was thrown out and why, for the startup banner.
  rpcEndpoints: rpcConfig.endpoints,
  rpcRejected: rpcConfig.rejected,
  get rpc() {
    return this.rpcEndpoints[0];
  },
  hermesBaseUrl: process.env.HERMES_BASE_URL ?? "https://hermes.pyth.network",
  feedId:
    process.env.PYTH_FEED_ID ??
    "ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d",
  receiverProgramId: new PublicKey(
    process.env.PYTH_RECEIVER_PROGRAM_ID ??
      "rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ"
  ),
  // Matches DEFAULT_WORMHOLE_PROGRAM_ID in @pythnetwork/pyth-solana-receiver.
  // Only a fallback: receiver.mjs reads the authoritative id from the on-chain
  // receiver config PDA. Note this address and the one with the `z` dropped
  // BOTH decode to valid 32-byte keys, so a typo here does not throw — it
  // silently points at a different program.
  wormholeProgramId: new PublicKey(
    process.env.WORMHOLE_PROGRAM_ID ??
      "HDwcJBJXjL9FpJ7UBsYBtaDjsBUhuLCUYoz3zr8SWWaQ"
  ),
  wickProgramId: new PublicKey(
    process.env.WICK_PROGRAM_ID ?? "FRtyvM3xcFhL5FbukUdzaMV7t4pePiqxPvp2ZHwptBE"
  ),
  tickIntervalMs: Number(process.env.TICK_INTERVAL_MS ?? 5000),
  dryRun: (process.env.DRY_RUN ?? "1") !== "0",
  // §8.6 — where a delegated guard ticks. Kept separate from `rpcEndpoints`:
  // the ER only knows about accounts that have been delegated to it, so sending
  // base-layer traffic here fails rather than falling back.
  magicblockRouterUrl:
    process.env.MAGICBLOCK_ROUTER_URL ?? "https://devnet-router.magicblock.app",
  magicblockErUrl:
    process.env.MAGICBLOCK_ER_URL ?? "https://devnet.magicblock.app",
};

/**
 * dotenv does no shell expansion and keeps surrounding quotes, so a `.env`
 * written with `~`, `$HOME`, or quotes reaches us verbatim. Normalize rather
 * than fail on a path the user reasonably expected to work.
 */
function resolveKeypairPath(raw) {
  const home = process.env.HOME ?? homedir();
  return raw
    .trim()
    .replace(/^["']|["']$/g, "")
    .replace(/^~(?=\/|$)/, home)
    .replace(/\$\{?HOME\}?/g, home);
}

export function crankerKeypair() {
  const path = resolveKeypairPath(
    process.env.CRANKER_KEYPAIR ?? "~/.config/solana/id.json"
  );
  let raw;
  try {
    raw = JSON.parse(readFileSync(path, "utf8"));
  } catch (err) {
    throw new Error(
      `cannot read keypair at ${path} (${err.code ?? err.message}). Set CRANKER_KEYPAIR in cranker/.env`
    );
  }
  return Keypair.fromSecretKey(Uint8Array.from(raw));
}
