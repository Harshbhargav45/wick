import { PublicKey } from "@solana/web3.js";
import { Buffer as IsomorphicBuffer } from "node:buffer";
import { config } from "./config.mjs";

export const GUARD_SEED = "guard";
export const ROUTE_CONFIG_SEED = "route_config";

export function guardPda(venueOwner) {
  const [pda, bump] = PublicKey.findProgramAddressSync(
    [IsomorphicBuffer.from(GUARD_SEED), venueOwner.toBuffer()],
    config.wickProgramId
  );
  return { pda, bump };
}

export function routeConfigPda() {
  const [pda, bump] = PublicKey.findProgramAddressSync(
    [IsomorphicBuffer.from(ROUTE_CONFIG_SEED)],
    config.wickProgramId
  );
  return { pda, bump };
}
