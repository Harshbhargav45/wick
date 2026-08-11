/**
 * Prove the RPC pool before the loop depends on it.
 *
 *   node src/rpc-check.mjs
 *
 * Three failure modes are all silent at runtime and all fatal, so each is
 * checked explicitly here rather than inferred from a loop that "seems fine":
 *
 *   1. An endpoint is unreachable — costs a rotation, survivable.
 *   2. An endpoint is reachable but on the **wrong cluster** — answers every
 *      query successfully with data about a chain the guard does not live on,
 *      so `findGuards` returns nothing and the loop logs "no guard accounts
 *      found" forever. This is the one that looks like a bug in the program.
 *   3. Every endpoint is rejected at config time and the pool silently fell
 *      back to the rate-limited public floor.
 *
 * Exits non-zero when no endpoint can serve the configured cluster.
 */
import { config, cluster } from "./config.mjs";
import { logEndpointConfig, redact, verifyEndpoints } from "./rpc.mjs";

const rpcConfig = {
  cluster,
  endpoints: config.rpcEndpoints,
  rejected: config.rpcRejected,
};

logEndpointConfig(rpcConfig);
console.log("");

const results = await verifyEndpoints({
  endpoints: config.rpcEndpoints,
  expectedCluster: cluster,
});

let healthy = 0;
for (const r of results) {
  if (r.ok) {
    healthy += 1;
    console.log(`[rpc-check] OK    ${r.url} — ${r.cluster}`);
  } else {
    console.error(`[rpc-check] FAIL  ${r.url} — ${r.error}`);
  }
}

console.log("");
console.log(
  `[rpc-check] ${healthy}/${results.length} endpoints serve ${cluster}`
);

if (healthy === 0) {
  console.error(
    `[rpc-check] no usable endpoint for ${cluster}. Set HELIUS_API_KEY / QUICKNODE_RPC_URL for ${cluster}, or SOLANA_CLUSTER to the cluster your endpoints actually serve.`
  );
  process.exit(1);
}

if (healthy === 1 && results[0]?.url === redact(config.rpcEndpoints[0])) {
  // Not fatal, but the operator should know the pool has no redundancy: a
  // single 429 stream on the public floor is enough to degrade the guard.
  console.warn(
    `[rpc-check] only one healthy endpoint — a rate limit has nowhere to rotate to.`
  );
}
