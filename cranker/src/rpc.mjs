/**
 * Failover RPC pool.
 *
 * The public devnet endpoint rate-limits a 5s tick loop hard: each tick issues
 * a getLatestBlockhash, three sends, three confirms and a couple of account
 * reads, and once `429`s start the guard's staleness window
 * (MAX_TICK_AGE_SECS, 10s) closes before a tick lands — the guard flips
 * `degraded` and stops protecting. So endpoints are a prioritized list, not a
 * string, and a rate-limit or transport failure rotates to the next one
 * mid-flight.
 *
 * Ordering is deliberate: keyed endpoints (Helius, QuickNode) first because
 * they carry the throughput, the public endpoint last as a floor so a missing
 * key degrades the loop instead of stopping it.
 *
 * **Cluster safety.** A keyed endpoint is a URL, and the free dashboards hand
 * out a *mainnet* URL by default. Pointing a devnet cranker at mainnet does not
 * fail loudly — it silently reads a cluster where the guard PDA does not exist,
 * so `findGuards` returns nothing and the loop reports "no guard accounts
 * found" forever. Endpoints whose host names a different cluster than
 * `SOLANA_CLUSTER` are therefore rejected at resolve time with a named reason,
 * and `verifyEndpoints` proves the rest by genesis hash before the loop starts.
 *
 * No secret is ever hardcoded here: every endpoint comes from the environment,
 * and `redact` strips the credential before anything is logged.
 */
import { Connection } from "@solana/web3.js";

/** Public RPC floor per cluster. Always reachable, always rate-limited. */
export const PUBLIC_RPC = {
  "mainnet-beta": "https://api.mainnet-beta.solana.com",
  devnet: "https://api.devnet.solana.com",
  testnet: "https://api.testnet.solana.com",
};

/**
 * Genesis hash per cluster. This is the only identification that cannot be
 * spoofed by a hostname: a provider may serve devnet from a host with no
 * "devnet" in its name at all, and a mistyped mainnet URL looks identical to a
 * correct devnet one until a transaction is sent to the wrong chain.
 */
export const CLUSTER_GENESIS = {
  "mainnet-beta": "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d",
  devnet: "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG",
  testnet: "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY",
};

export const DEFAULT_CLUSTER = "devnet";

/** Errors where the request provably did not execute, so retrying is safe. */
const RETRYABLE = [
  /\b429\b/,
  /rate.?limit/i,
  /too many requests/i,
  /\b50[0234]\b/,
  /bad gateway/i,
  /service unavailable/i,
  /gateway timeout/i,
  /timeout/i,
  /timed out/i,
  /ETIMEDOUT/,
  /ECONNRESET/,
  /ECONNREFUSED/,
  /EAI_AGAIN/,
  /ENOTFOUND/,
  /socket hang up/i,
  /fetch failed/i,
  /network error/i,
  /failed to fetch/i,
];

export function isRetryable(err) {
  const msg = `${err?.message ?? err} ${err?.cause?.message ?? ""}`;
  return RETRYABLE.some((re) => re.test(msg));
}

/**
 * Cluster named by an endpoint's host, or `null` when the host says nothing.
 *
 * Hostname is a hint, not proof — `verifyEndpoints` is the proof. It is still
 * worth checking, because it is the one check available *before* any network
 * call, and it catches the overwhelmingly common mistake: pasting the mainnet
 * URL a provider dashboard shows first into a devnet deployment.
 */
export function clusterFromUrl(url) {
  let host;
  try {
    host = new URL(url).hostname.toLowerCase();
  } catch {
    return null;
  }
  // `mainnet` must be tested first: "solana-mainnet.quiknode.pro" contains
  // neither "devnet" nor "testnet", but "devnet.magicblock.app" would match a
  // naive substring search for "net".
  if (host.includes("mainnet")) return "mainnet-beta";
  if (host.includes("devnet")) return "devnet";
  if (host.includes("testnet")) return "testnet";
  if (host === "api.solana.com") return "mainnet-beta";
  return null;
}

/**
 * Build the candidate endpoint list from environment, in priority order.
 *
 * `SOLANA_RPC_ENDPOINTS` (comma-separated) wins outright when set; otherwise
 * the keyed providers are assembled from their own variables so a `.env` only
 * needs the key, not the whole URL.
 */
function candidateEndpoints(env, cluster) {
  const explicit = (env.SOLANA_RPC_ENDPOINTS ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
  if (explicit.length > 0) {
    return { list: dedupe(explicit), explicit: true };
  }

  const list = [];
  if (env.HELIUS_API_KEY) {
    // The Helius host is cluster-scoped, so the key alone is enough — but only
    // if the host is built for the cluster we are actually running on.
    const host =
      cluster === "mainnet-beta"
        ? "mainnet.helius-rpc.com"
        : `${cluster}.helius-rpc.com`;
    list.push(`https://${host}/?api-key=${env.HELIUS_API_KEY.trim()}`);
  }
  if (env.HELIUS_RPC_URL) list.push(env.HELIUS_RPC_URL.trim());
  if (env.QUICKNODE_RPC_URL) list.push(env.QUICKNODE_RPC_URL.trim());
  if (env.SOLANA_RPC) list.push(env.SOLANA_RPC.trim());
  list.push(PUBLIC_RPC[cluster]);
  return { list: dedupe(list), explicit: false };
}

/**
 * Resolve the endpoint list together with the diagnostics needed to explain it.
 *
 * Returns `{ cluster, endpoints, rejected }`. `rejected` entries carry a
 * redacted URL and a reason, so a dropped endpoint is reported rather than
 * silently missing from a shorter-than-expected list.
 *
 * An explicit `SOLANA_RPC_ENDPOINTS` list is still cluster-checked: the whole
 * point of the check is that the operator did not notice the cluster, and
 * "I typed it out by hand" is not evidence they did.
 */
export function resolveEndpointConfig(env = process.env) {
  const cluster = (env.SOLANA_CLUSTER ?? DEFAULT_CLUSTER).trim();
  if (!PUBLIC_RPC[cluster]) {
    throw new Error(
      `SOLANA_CLUSTER="${cluster}" is not one of ${Object.keys(PUBLIC_RPC).join(", ")}`
    );
  }

  const { list } = candidateEndpoints(env, cluster);
  const endpoints = [];
  const rejected = [];

  for (const url of list) {
    if (!/^https?:\/\//i.test(url)) {
      rejected.push({ url: redact(url), reason: "not an http(s) URL" });
      continue;
    }
    const named = clusterFromUrl(url);
    if (named !== null && named !== cluster) {
      rejected.push({
        url: redact(url),
        reason: `host names the ${named} cluster, but SOLANA_CLUSTER=${cluster}`,
      });
      continue;
    }
    endpoints.push(url);
  }

  // The public floor is unconditional: a config where every keyed endpoint was
  // rejected must still be able to run, degraded, rather than throw at startup.
  if (!endpoints.includes(PUBLIC_RPC[cluster])) {
    endpoints.push(PUBLIC_RPC[cluster]);
  }

  return { cluster, endpoints, rejected };
}

/** Endpoint list only — the shape every existing call site expects. */
export function resolveEndpoints(env = process.env) {
  return resolveEndpointConfig(env).endpoints;
}

function dedupe(list) {
  return [...new Set(list.filter(Boolean))];
}

/** Strip the API key before logging — these URLs are credentials. */
export function redact(url) {
  return String(url)
    .replace(/(api-key=)[^&]+/i, "$1***")
    .replace(/(\/\/[^/]+\/)[A-Za-z0-9_-]{16,}/, "$1***");
}

/**
 * A `Connection`-shaped object that transparently rotates endpoints.
 *
 * Returned as a Proxy so the existing `new Connection(...)` call sites and
 * Anchor's `AnchorProvider` keep working unchanged: property reads land on the
 * live connection, and method calls go through the retry loop.
 *
 * Retrying a `sendTransaction` is safe here because the transaction is already
 * signed — a resend carries the same signature, which the cluster dedupes.
 */
export function createFailoverConnection(opts = {}) {
  const commitment = opts.commitment ?? "confirmed";
  const endpoints = opts.endpoints ?? resolveEndpoints();
  const maxAttempts = opts.maxAttempts ?? endpoints.length * 2;
  const baseDelayMs = opts.baseDelayMs ?? 250;
  const log = opts.log ?? ((m) => console.warn(m));
  // Injectable so the failover path itself is testable without a live cluster:
  // the tests hand in a factory that returns scripted stubs, and the rotation,
  // backoff and give-up behaviour are exercised for real rather than asserted
  // about from the configuration alone.
  const connect = opts.connect ?? ((url) => new Connection(url, commitment));
  const sleep = opts.sleep ?? ((ms) => new Promise((r) => setTimeout(r, ms)));

  if (endpoints.length === 0) throw new Error("no RPC endpoints configured");

  const cache = new Map();
  let index = 0;
  // Per-endpoint counters. Failover that is working looks exactly like failover
  // that is not: both produce a stream of successful ticks. The counters are
  // what makes "we have silently been on the public floor for an hour"
  // observable — `logStats` prints them and `__stats` exposes them.
  const stats = endpoints.map((url) => ({
    url: redact(url),
    ok: 0,
    failed: 0,
    rotations: 0,
  }));

  const conn = (i) => {
    const url = endpoints[i];
    if (!cache.has(url)) cache.set(url, connect(url, commitment));
    return cache.get(url);
  };

  async function call(prop, args) {
    let lastErr;
    for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
      const i = index;
      try {
        const target = conn(i);
        const out = await Reflect.get(target, prop, target).apply(target, args);
        stats[i].ok += 1;
        return out;
      } catch (err) {
        lastErr = err;
        stats[i].failed += 1;
        if (!isRetryable(err)) throw err;
        // Rotate only if nobody else already moved us off this endpoint.
        if (index === i) {
          index = (i + 1) % endpoints.length;
          stats[i].rotations += 1;
        }
        log(
          `[rpc] ${String(prop)} failed on ${redact(endpoints[i])} (${
            err.message ?? err
          }) → ${redact(endpoints[index])}`
        );
        // Jitter so a restarted fleet does not resynchronize onto one endpoint.
        const delay = baseDelayMs * 2 ** Math.min(attempt, 4);
        await sleep(delay / 2 + Math.random() * delay);
      }
    }
    // Every endpoint has been tried and every one failed. Surface the last
    // error rather than a synthesized one: the caller needs the actual reason.
    throw lastErr;
  }

  return new Proxy(
    // The proxy target is a real Connection so `instanceof Connection` and
    // prototype walks still hold — Anchor builds a Provider around this object.
    conn(0),
    {
      get(_t, prop) {
        if (prop === "__endpoints") return endpoints.map(redact);
        if (prop === "__activeEndpoint") return redact(endpoints[index]);
        if (prop === "__stats") return stats.map((s) => ({ ...s }));
        const target = conn(index);
        const value = Reflect.get(target, prop, target);
        if (typeof value !== "function") return value;
        return (...args) => call(prop, args);
      },
      has(_t, prop) {
        return prop in conn(index);
      },
    }
  );
}

/**
 * Prove each endpoint before the loop depends on it.
 *
 * Reachability and cluster identity are both checked, because the two failure
 * modes are different and both are silent: an unreachable endpoint just costs a
 * rotation, but a *reachable endpoint on the wrong cluster* answers every query
 * successfully with data about a chain the guard does not live on.
 *
 * Returns `{ url, ok, cluster, error }` per endpoint. Nothing is mutated and
 * nothing throws: the caller decides whether a partially-healthy pool is fatal.
 * `expectedCluster` mismatches are reported as `ok: false` with a reason.
 */
export async function verifyEndpoints({
  endpoints = resolveEndpoints(),
  expectedCluster = DEFAULT_CLUSTER,
  commitment = "confirmed",
  connect = (url) => new Connection(url, commitment),
  timeoutMs = 8_000,
} = {}) {
  const expectedGenesis = CLUSTER_GENESIS[expectedCluster];

  return Promise.all(
    endpoints.map(async (url) => {
      const result = { url: redact(url), ok: false, cluster: null, error: null };
      try {
        const conn = connect(url, commitment);
        const genesis = await withTimeout(conn.getGenesisHash(), timeoutMs);
        result.cluster =
          Object.entries(CLUSTER_GENESIS).find(([, h]) => h === genesis)?.[0] ??
          `unknown (${genesis})`;
        if (expectedGenesis && genesis !== expectedGenesis) {
          result.error = `serves ${result.cluster}, expected ${expectedCluster}`;
          return result;
        }
        result.ok = true;
      } catch (err) {
        result.error = err?.message ?? String(err);
      }
      return result;
    })
  );
}

function withTimeout(promise, ms) {
  let timer;
  return Promise.race([
    promise.finally(() => clearTimeout(timer)),
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`timed out after ${ms}ms`)), ms);
    }),
  ]);
}

let shared = null;

/**
 * One pool per process. Rotation state is the point of sharing it: when the
 * tick loop's send gets a `429`, the receiver's Anchor provider and the
 * post-vaa helper should already be on the next endpoint rather than each
 * discovering the same rate limit on its own.
 */
export function sharedConnection(opts = {}) {
  if (!shared) shared = createFailoverConnection(opts);
  return shared;
}

/** Drop the process-wide pool. Tests only — nothing in the loop should call it. */
export function resetSharedConnection() {
  shared = null;
}

/**
 * One-line startup banner covering the whole pool, including what was thrown
 * out and why. Called by every entry point so a misconfigured endpoint is
 * reported at the top of the log instead of being inferred from behaviour.
 */
export function logEndpointConfig(config, log = console.log) {
  log(`[rpc] cluster=${config.cluster} endpoints=${config.endpoints.map(redact).join(" → ")}`);
  for (const r of config.rejected) {
    log(`[rpc] REJECTED ${r.url} — ${r.reason}`);
  }
}
