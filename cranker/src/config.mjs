import "dotenv/config";
import { PublicKey } from "@solana/web3.js";
import { readFileSync } from "node:fs";
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

export function crankerKeypair() {
  const path =
    process.env.CRANKER_KEYPAIR ??
    "/home/harsh/Downloads/outreach_pipeline/timekeeper/wallet.json";
  const raw = JSON.parse(readFileSync(path, "utf8"));
  return Keypair.fromSecretKey(Uint8Array.from(raw));
}
