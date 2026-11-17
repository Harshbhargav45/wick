/**
 * Solana Explorer links for the addresses and signatures the console shows.
 *
 * The cluster comes from `NEXT_PUBLIC_SOLANA_CLUSTER` (the same value the RPC
 * config already uses) rather than being inferred from an endpoint string: a
 * keyed Helius/QuickNode URL does not spell out devnet vs mainnet. Mainnet gets
 * the bare URL because that is Explorer's default; everything else carries an
 * explicit `?cluster=` so a devnet signature never opens against mainnet.
 */

const EXPLORER = 'https://explorer.solana.com';
const MAINNET = 'mainnet-beta';

export type SolanaCluster = 'devnet' | 'testnet' | 'mainnet-beta' | string;

export function cluster(): SolanaCluster {
  // Written out literally because Next inlines `process.env.NEXT_PUBLIC_*` by
  // textual substitution at build time.
  const raw = (process.env.NEXT_PUBLIC_SOLANA_CLUSTER ?? 'devnet').trim();
  if (!raw) return 'devnet';
  if (raw === 'mainnet') return MAINNET;
  return raw;
}

function url(kind: 'address' | 'tx', value: string): string {
  const base = `${EXPLORER}/${kind}/${encodeURIComponent(value)}`;
  const c = cluster();
  return c === MAINNET ? base : `${base}?cluster=${encodeURIComponent(c)}`;
}

export function explorerAccountUrl(address: string): string {
  return url('address', address);
}

export function explorerTxUrl(signature: string): string {
  return url('tx', signature);
}
