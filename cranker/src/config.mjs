import "dotenv/config";
import { PublicKey } from "@solana/web3.js";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { Keypair } from "@solana/web3.js";

export const config = {
  rpc: process.env.SOLANA_RPC ?? "https://api.devnet.solana.com",
  hermesBaseUrl: process.env.HERMES_BASE_URL ?? "https://hermes.pyth.network",
  feedId:
    process.env.PYTH_FEED_ID ??
    "ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d",
  receiverProgramId: new PublicKey(
    process.env.PYTH_RECEIVER_PROGRAM_ID ??
      "rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ"
  ),
  wormholeProgramId: new PublicKey(
    process.env.WORMHOLE_PROGRAM_ID ??
      "HDwcJBJXjL9FpJ7UBsYBtaDjsBUhuLCUYoz3r8SWWaQ"
  ),
  wickProgramId: new PublicKey(
    process.env.WICK_PROGRAM_ID ?? "FRtyvM3xcFhL5FbukUdzaMV7t4pePiqxPvp2ZHwptBE"
  ),
  tickIntervalMs: Number(process.env.TICK_INTERVAL_MS ?? 5000),
  dryRun: (process.env.DRY_RUN ?? "1") !== "0",
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
