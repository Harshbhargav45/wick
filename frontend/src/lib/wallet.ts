/**
 * Minimal injected-wallet bridge (Phantom / Solflare / Backpack). No wallet
 * adapter dependency — the console only ever needs `connect`, the public
 * key, and `signAndSendTransaction`, which every injected provider exposes.
 */

import { PublicKey, type Transaction, type VersionedTransaction } from '@solana/web3.js';

interface InjectedProvider {
  isPhantom?: boolean;
  publicKey?: { toBytes(): Uint8Array } | null;
  connect(): Promise<{ publicKey: { toBytes(): Uint8Array } }>;
  disconnect?(): Promise<void>;
  signAndSendTransaction(
    tx: Transaction | VersionedTransaction,
  ): Promise<{ signature: string }>;
  /**
   * Sign without sending. Optional — not every injected provider exposes it,
   * which is why the 2-of-2 hand-off falls back to an unsigned export rather
   * than assuming it is there.
   */
  signTransaction?(tx: Transaction): Promise<Transaction>;
  on?(event: string, handler: () => void): void;
  removeListener?(event: string, handler: () => void): void;
}

declare global {
  interface Window {
    solana?: InjectedProvider;
    backpack?: InjectedProvider;
  }
}

function findProvider(): InjectedProvider | null {
  if (typeof window === 'undefined') return null;
  return window.solana ?? window.backpack ?? null;
}

export interface WalletState {
  connected: boolean;
  publicKey: PublicKey | null;
  providerName: string | null;
}

const listeners = new Set<() => void>();
let state: WalletState = { connected: false, publicKey: null, providerName: null };

function emit(next: WalletState) {
  state = next;
  for (const l of listeners) l();
}

function attachProviderListeners(provider: InjectedProvider) {
  provider.on?.('connect', () => {
    if (provider.publicKey) {
      emit({
        connected: true,
        publicKey: new PublicKey(provider.publicKey.toBytes()),
        providerName: provider.isPhantom ? 'Phantom' : 'Wallet',
      });
    }
  });
  provider.on?.('disconnect', () => {
    emit({ connected: false, publicKey: null, providerName: null });
  });
}

let attached = false;

export function subscribeWallet(onChange: () => void): () => void {
  listeners.add(onChange);
  const provider = findProvider();
  if (provider && !attached) {
    attached = true;
    attachProviderListeners(provider);
    if (provider.publicKey) {
      emit({
        connected: true,
        publicKey: new PublicKey(provider.publicKey.toBytes()),
        providerName: provider.isPhantom ? 'Phantom' : 'Wallet',
      });
    }
  }
  return () => listeners.delete(onChange);
}

export function getWalletSnapshot(): WalletState {
  return state;
}

export function getWalletServerSnapshot(): WalletState {
  return { connected: false, publicKey: null, providerName: null };
}

export function isWalletAvailable(): boolean {
  return findProvider() !== null;
}

export async function connectWallet(): Promise<void> {
  const provider = findProvider();
  if (!provider) throw new Error('No Solana wallet found (Phantom, Solflare, or Backpack).');
  if (!attached) {
    attached = true;
    attachProviderListeners(provider);
  }
  const res = await provider.connect();
  emit({
    connected: true,
    publicKey: new PublicKey(res.publicKey.toBytes()),
    providerName: provider.isPhantom ? 'Phantom' : 'Wallet',
  });
}

export async function disconnectWallet(): Promise<void> {
  const provider = findProvider();
  await provider?.disconnect?.();
  emit({ connected: false, publicKey: null, providerName: null });
}

/** Sign and send with the injected provider; returns the tx signature. */
export async function sendWithWallet(tx: Transaction): Promise<string> {
  const provider = findProvider();
  if (!provider) throw new Error('No Solana wallet found.');
  const { signature } = await provider.signAndSendTransaction(tx);
  return signature;
}

/** True when the provider can sign without sending — needed for 2-of-2 hand-off. */
export function canPartialSign(): boolean {
  return typeof findProvider()?.signTransaction === 'function';
}

/**
 * Add this wallet's signature and return the transaction, still unsent.
 *
 * This is the owner's half of a 2-of-2: the co-authority signs the same bytes
 * elsewhere, so what comes back is deliberately incomplete. The console
 * serializes it with `requireAllSignatures: false` and hands it over rather than
 * broadcasting — a transaction sent one signature short is rejected by the
 * program, and retrying it faster does not help.
 */
export async function partialSignWithWallet(tx: Transaction): Promise<Transaction> {
  const provider = findProvider();
  if (!provider) throw new Error('No Solana wallet found.');
  if (!provider.signTransaction) {
    throw new Error(
      'This wallet cannot sign without sending, so the owner half of a 2-of-2 cannot be produced here. Export the unsigned transaction and co-sign it offline.',
    );
  }
  return provider.signTransaction(tx);
}
