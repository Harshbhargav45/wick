import assert from "node:assert/strict";
import { test } from "node:test";
import {
  CLUSTER_GENESIS,
  PUBLIC_RPC,
  clusterFromUrl,
  createFailoverConnection,
  isRetryable,
  redact,
  resolveEndpointConfig,
  resolveEndpoints,
  verifyEndpoints,
} from "../src/rpc.mjs";

const PUBLIC = PUBLIC_RPC.devnet;

// ---------------------------------------------------------------------------
// Retry classification
// ---------------------------------------------------------------------------

test("a 429 is retryable, a program error is not", () => {
  assert.equal(isRetryable(new Error("429 Too Many Requests")), true);
  assert.equal(isRetryable(new Error("server responded with 503")), true);
  assert.equal(isRetryable(new Error("socket hang up")), true);
  // The whole point of the allow-list: a failed transaction must surface, not
  // get replayed against the next endpoint.
  assert.equal(isRetryable(new Error("custom program error: 0x1771")), false);
  assert.equal(isRetryable(new Error("blockhash not found")), false);
});

test("a retryable cause is seen through the wrapper", () => {
  const err = new Error("fetch failed", { cause: new Error("ETIMEDOUT") });
  assert.equal(isRetryable(err), true);
});

// ---------------------------------------------------------------------------
// Endpoint resolution
// ---------------------------------------------------------------------------

test("keyed endpoints come first and the public floor is last", () => {
  const list = resolveEndpoints({
    HELIUS_API_KEY: "abc",
    QUICKNODE_RPC_URL: "https://x.solana-devnet.quiknode.pro/deadbeef/",
  });
  assert.equal(list.length, 3);
  assert.match(list[0], /devnet\.helius-rpc\.com/);
  assert.match(list[1], /quiknode\.pro/);
  assert.equal(list[2], PUBLIC);
});

test("an explicit list wins outright", () => {
  const list = resolveEndpoints({
    SOLANA_RPC_ENDPOINTS: " https://rpc-a.example , https://rpc-b.example ",
    HELIUS_API_KEY: "abc",
  });
  // The public floor is appended unconditionally, so a fully-rejected or
  // fully-unreachable explicit list can still run degraded rather than throw.
  assert.deepEqual(list, [
    "https://rpc-a.example",
    "https://rpc-b.example",
    PUBLIC,
  ]);
});

test("with nothing configured the loop still has an endpoint", () => {
  assert.deepEqual(resolveEndpoints({}), [PUBLIC]);
});

test("duplicates collapse so a rotation always moves to a different host", () => {
  assert.deepEqual(resolveEndpoints({ SOLANA_RPC: PUBLIC }), [PUBLIC]);
});

test("the helius host follows the configured cluster", () => {
  const devnet = resolveEndpoints({ HELIUS_API_KEY: "k" });
  assert.match(devnet[0], /^https:\/\/devnet\.helius-rpc\.com\//);
  const mainnet = resolveEndpoints({
    HELIUS_API_KEY: "k",
    SOLANA_CLUSTER: "mainnet-beta",
  });
  assert.match(mainnet[0], /^https:\/\/mainnet\.helius-rpc\.com\//);
});

// ---------------------------------------------------------------------------
// Cluster safety — the failure that looks like a program bug
// ---------------------------------------------------------------------------

test("clusterFromUrl reads the cluster out of the host", () => {
  assert.equal(
    clusterFromUrl("https://mainnet.helius-rpc.com/?api-key=x"),
    "mainnet-beta"
  );
  assert.equal(
    clusterFromUrl("https://y.solana-mainnet.quiknode.pro/abc/"),
    "mainnet-beta"
  );
  assert.equal(clusterFromUrl("https://api.devnet.solana.com"), "devnet");
  assert.equal(clusterFromUrl("https://api.testnet.solana.com"), "testnet");
  // A host that names no cluster is not a rejection — it is simply unknown,
  // and verifyEndpoints settles it by genesis hash.
  assert.equal(clusterFromUrl("https://rpc.example.com"), null);
  assert.equal(clusterFromUrl("not a url"), null);
});

test("a mainnet endpoint is rejected on a devnet cranker, with a reason", () => {
  const cfg = resolveEndpointConfig({
    HELIUS_RPC_URL: "https://mainnet.helius-rpc.com/?api-key=secret",
    QUICKNODE_RPC_URL: "https://y.solana-mainnet.quiknode.pro/abc0123456789def/",
  });
  assert.equal(cfg.cluster, "devnet");
  // Both keyed endpoints served mainnet, so the pool is the public floor only.
  assert.deepEqual(cfg.endpoints, [PUBLIC]);
  assert.equal(cfg.rejected.length, 2);
  for (const r of cfg.rejected) {
    assert.match(r.reason, /mainnet-beta cluster, but SOLANA_CLUSTER=devnet/);
    // The reason is logged, so it must not carry the credential.
    assert.doesNotMatch(r.url, /secret|abc0123456789def/);
  }
});

test("the same mainnet endpoints are accepted when the cluster says mainnet", () => {
  const cfg = resolveEndpointConfig({
    HELIUS_RPC_URL: "https://mainnet.helius-rpc.com/?api-key=secret",
    SOLANA_CLUSTER: "mainnet-beta",
  });
  assert.deepEqual(cfg.rejected, []);
  assert.equal(cfg.endpoints[0], "https://mainnet.helius-rpc.com/?api-key=secret");
  assert.equal(cfg.endpoints.at(-1), PUBLIC_RPC["mainnet-beta"]);
});

test("a garbage endpoint is dropped rather than crashing the pool", () => {
  const cfg = resolveEndpointConfig({ SOLANA_RPC_ENDPOINTS: "ws://nope,,   " });
  assert.deepEqual(cfg.endpoints, [PUBLIC]);
  assert.equal(cfg.rejected[0].reason, "not an http(s) URL");
});

test("an unknown cluster name is a startup error, not a silent default", () => {
  assert.throws(
    () => resolveEndpointConfig({ SOLANA_CLUSTER: "mainnet" }),
    /not one of/
  );
});

// ---------------------------------------------------------------------------
// Redaction
// ---------------------------------------------------------------------------

test("redact strips api keys and path secrets", () => {
  assert.equal(
    redact("https://devnet.helius-rpc.com/?api-key=s3cret"),
    "https://devnet.helius-rpc.com/?api-key=***"
  );
  assert.equal(
    redact("https://x.solana-devnet.quiknode.pro/abcdef0123456789abcdef/"),
    "https://x.solana-devnet.quiknode.pro/***/"
  );
});

// ---------------------------------------------------------------------------
// Actual failover behaviour
//
// These drive the real `createFailoverConnection` with scripted stubs, so the
// rotation, the give-up condition and the "do not retry a program error" rule
// are exercised rather than asserted about from the configuration alone.
// ---------------------------------------------------------------------------

/** A stub Connection whose every call runs the scripted behaviour for its URL. */
function stubPool(script) {
  const calls = [];
  const connect = (url) => ({
    url,
    getSlot: async (...args) => {
      calls.push({ url, method: "getSlot", args });
      const behaviour = script[url];
      if (typeof behaviour === "function") return behaviour();
      throw new Error(`no script for ${url}`);
    },
    getGenesisHash: async () => {
      calls.push({ url, method: "getGenesisHash" });
      return script[url]();
    },
  });
  return { connect, calls };
}

const A = "https://a.example";
const B = "https://b.example";
const C = "https://c.example";

test("a rate-limited endpoint rotates to the next and the call still succeeds", async () => {
  let aHits = 0;
  const { connect, calls } = stubPool({
    [A]: () => {
      aHits += 1;
      throw new Error("429 Too Many Requests");
    },
    [B]: () => 12345,
  });
  const conn = createFailoverConnection({
    endpoints: [A, B],
    connect,
    sleep: async () => {},
    log: () => {},
  });

  assert.equal(await conn.getSlot(), 12345);
  assert.equal(aHits, 1);
  assert.deepEqual(
    calls.map((c) => c.url),
    [A, B]
  );
  // The pool stays on the healthy endpoint rather than reverting to the one
  // that just rate-limited it.
  assert.equal(conn.__activeEndpoint, B);
});

test("arguments survive the rotation intact", async () => {
  const { connect, calls } = stubPool({
    [A]: () => {
      throw new Error("503 service unavailable");
    },
    [B]: () => "ok",
  });
  const conn = createFailoverConnection({
    endpoints: [A, B],
    connect,
    sleep: async () => {},
    log: () => {},
  });
  await conn.getSlot("confirmed", { minContextSlot: 7 });
  for (const call of calls) {
    assert.deepEqual(call.args, ["confirmed", { minContextSlot: 7 }]);
  }
});

test("a program error is surfaced immediately, never replayed elsewhere", async () => {
  const { connect, calls } = stubPool({
    [A]: () => {
      throw new Error("custom program error: 0x1771");
    },
    [B]: () => 1,
  });
  const conn = createFailoverConnection({
    endpoints: [A, B],
    connect,
    sleep: async () => {},
    log: () => {},
  });

  await assert.rejects(() => conn.getSlot(), /custom program error/);
  // The decisive assertion: B was never asked. Replaying a landed-but-failed
  // transaction against a second endpoint is how a double-send happens.
  assert.deepEqual(
    calls.map((c) => c.url),
    [A]
  );
});

test("when every endpoint is down the last error surfaces — no silent success", async () => {
  const { connect, calls } = stubPool({
    [A]: () => {
      throw new Error("ECONNREFUSED");
    },
    [B]: () => {
      throw new Error("ETIMEDOUT");
    },
  });
  const conn = createFailoverConnection({
    endpoints: [A, B],
    connect,
    sleep: async () => {},
    log: () => {},
  });

  await assert.rejects(() => conn.getSlot(), /ETIMEDOUT|ECONNREFUSED/);
  // maxAttempts defaults to endpoints.length * 2, so the pool is swept twice
  // before giving up rather than failing on the first pass.
  assert.equal(calls.length, 4);
});

test("rotation wraps around the whole pool", async () => {
  const { connect } = stubPool({
    [A]: () => {
      throw new Error("429");
    },
    [B]: () => {
      throw new Error("429");
    },
    [C]: () => "third",
  });
  const conn = createFailoverConnection({
    endpoints: [A, B, C],
    connect,
    sleep: async () => {},
    log: () => {},
  });
  assert.equal(await conn.getSlot(), "third");
  assert.equal(conn.__activeEndpoint, C);
});

test("per-endpoint counters make a silent fallback observable", async () => {
  const { connect } = stubPool({
    [A]: () => {
      throw new Error("429");
    },
    [B]: () => 1,
  });
  const conn = createFailoverConnection({
    endpoints: [A, B],
    connect,
    sleep: async () => {},
    log: () => {},
  });
  await conn.getSlot();
  await conn.getSlot();

  const stats = conn.__stats;
  assert.equal(stats[0].failed, 1);
  assert.equal(stats[0].rotations, 1);
  assert.equal(stats[1].ok, 2);
  // Counters are a copy — a caller cannot corrupt the pool's own accounting.
  stats[1].ok = 999;
  assert.equal(conn.__stats[1].ok, 2);
});

test("the failover log redacts the credential", async () => {
  const keyed = "https://devnet.helius-rpc.com/?api-key=s3cret";
  const lines = [];
  const { connect } = stubPool({
    [keyed]: () => {
      throw new Error("429");
    },
    [PUBLIC]: () => 1,
  });
  const conn = createFailoverConnection({
    endpoints: [keyed, PUBLIC],
    connect,
    sleep: async () => {},
    log: (m) => lines.push(m),
  });
  await conn.getSlot();

  assert.equal(lines.length, 1);
  assert.doesNotMatch(lines[0], /s3cret/);
  assert.match(lines[0], /api-key=\*\*\*/);
  assert.match(lines[0], /→ https:\/\/api\.devnet\.solana\.com/);
});

// ---------------------------------------------------------------------------
// Endpoint verification
// ---------------------------------------------------------------------------

test("verifyEndpoints passes an endpoint serving the expected cluster", async () => {
  const { connect } = stubPool({ [A]: () => CLUSTER_GENESIS.devnet });
  const [result] = await verifyEndpoints({
    endpoints: [A],
    expectedCluster: "devnet",
    connect,
  });
  assert.equal(result.ok, true);
  assert.equal(result.cluster, "devnet");
});

test("verifyEndpoints catches a reachable endpoint on the wrong chain", async () => {
  const { connect } = stubPool({ [A]: () => CLUSTER_GENESIS["mainnet-beta"] });
  const [result] = await verifyEndpoints({
    endpoints: [A],
    expectedCluster: "devnet",
    connect,
  });
  assert.equal(result.ok, false);
  assert.equal(result.cluster, "mainnet-beta");
  assert.match(result.error, /serves mainnet-beta, expected devnet/);
});

test("verifyEndpoints reports an unreachable endpoint without throwing", async () => {
  const { connect } = stubPool({
    [A]: () => {
      throw new Error("ENOTFOUND");
    },
    [B]: () => CLUSTER_GENESIS.devnet,
  });
  const results = await verifyEndpoints({
    endpoints: [A, B],
    expectedCluster: "devnet",
    connect,
  });
  assert.equal(results[0].ok, false);
  assert.match(results[0].error, /ENOTFOUND/);
  // One bad endpoint does not mask the healthy one behind it.
  assert.equal(results[1].ok, true);
});

test("verifyEndpoints times out rather than hanging the preflight", async () => {
  const connect = () => ({
    getGenesisHash: () => new Promise(() => {}),
  });
  const [result] = await verifyEndpoints({
    endpoints: [A],
    expectedCluster: "devnet",
    connect,
    timeoutMs: 20,
  });
  assert.equal(result.ok, false);
  assert.match(result.error, /timed out after 20ms/);
});

test("verifyEndpoints redacts the endpoint it reports on", async () => {
  const keyed = "https://devnet.helius-rpc.com/?api-key=s3cret";
  const { connect } = stubPool({ [keyed]: () => CLUSTER_GENESIS.devnet });
  const [result] = await verifyEndpoints({
    endpoints: [keyed],
    expectedCluster: "devnet",
    connect,
  });
  assert.doesNotMatch(result.url, /s3cret/);
});
