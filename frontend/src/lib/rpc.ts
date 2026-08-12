import { Connection } from '@solana/web3.js';

const PUBLIC_DEVNET = 'https://api.devnet.solana.com';

/**
 * Errors where the request provably did not execute, so retrying is safe.
 * Mirrors `cranker/src/rpc.mjs`.
 */
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
  /failed to fetch/i,
  /fetch failed/i,
  /network ?error/i,
  /load failed/i,
];

export function isRetryable(err: unknown): boolean {
  const e = err as { message?: string; cause?: { message?: string } };
  const msg = `${e?.message ?? String(err)} ${e?.cause?.message ?? ''}`;
  return RETRYABLE.some((re) => re.test(msg));
}

/**
 * Prioritized endpoint list.
 *
 * Every reference to `process.env.NEXT_PUBLIC_*` has to be written out
 * literally: Next inlines these at build time by textual substitution, so a
 * dynamic `process.env[name]` lookup is left as-is and resolves to undefined in
 * the browser.
 */
export function rpcEndpoints(): string[] {
  const explicit = (process.env.NEXT_PUBLIC_SOLANA_RPC_ENDPOINTS ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
  if (explicit.length > 0) return [...new Set(explicit)];

  const list = [
    process.env.NEXT_PUBLIC_HELIUS_RPC_URL,
    process.env.NEXT_PUBLIC_QUICKNODE_RPC_URL,
    process.env.NEXT_PUBLIC_SOLANA_RPC,
    PUBLIC_DEVNET,
  ];
  return [...new Set(list.map((s) => s?.trim()).filter((s): s is string => !!s))];
}

/** Strip the API key before rendering an endpoint in the UI. */
export function redactRpc(url: string): string {
  return url
    .replace(/(api-key=)[^&]+/i, '$1***')
    .replace(/(\/\/[^/]+\/)[A-Za-z0-9_-]{16,}/, '$1***');
}

/**
 * A `Connection` that rotates endpoints on rate limits.
 *
 * The console polls every 5s and each poll issues three requests, which is
 * enough to get 429'd off public devnet with a couple of tabs open. Without
 * failover the panel just shows a stale snapshot and a fetch error.
 */
export function createFailoverConnection(
  endpoints: string[] = rpcEndpoints(),
  commitment: 'confirmed' | 'processed' | 'finalized' = 'confirmed',
): Connection {
  if (endpoints.length === 0) throw new Error('no RPC endpoints configured');

  const cache = new Map<string, Connection>();
  const maxAttempts = endpoints.length * 2;
  let index = 0;

  const conn = (i: number): Connection => {
    const url = endpoints[i];
    let c = cache.get(url);
    if (!c) {
      c = new Connection(url, commitment);
      cache.set(url, c);
    }
    return c;
  };

  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

  return new Proxy(conn(0), {
    get(_target, prop) {
      const active = conn(index);
      const value = Reflect.get(active, prop, active);
      if (typeof value !== 'function') return value;
      return async (...args: unknown[]) => {
        let lastErr: unknown;
        for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
          const i = index;
          try {
            const target = conn(i);
            const fn = Reflect.get(target, prop, target) as (...a: unknown[]) => unknown;
            return await fn.apply(target, args);
          } catch (err) {
            lastErr = err;
            if (!isRetryable(err)) throw err;
            if (index === i) index = (i + 1) % endpoints.length;
            await sleep(200 * 2 ** Math.min(attempt, 4) * (0.5 + Math.random()));
          }
        }
        throw lastErr;
      };
    },
  }) as Connection;
}
