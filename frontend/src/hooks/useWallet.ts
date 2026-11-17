'use client';

import { useSyncExternalStore } from 'react';
import {
  getWalletServerSnapshot,
  getWalletSnapshot,
  isWalletAvailable,
  subscribeWallet,
  type WalletState,
} from '@/lib/wallet';

/**
 * Reads the injected wallet through `useSyncExternalStore` so connect events
 * fired by the extension land without a setState-in-effect.
 */
export function useWallet(): WalletState {
  return useSyncExternalStore(subscribeWallet, getWalletSnapshot, getWalletServerSnapshot);
}

/** Injection does not fire a DOM event we can rely on, so this never notifies. */
function subscribeAvailability(): () => void {
  return () => {};
}

/**
 * Whether an injected provider exists.
 *
 * The server snapshot is `false`, so hydration agrees with the server and React
 * re-reads the live snapshot on the next render. Reading `window` during the
 * initial render instead would make the first client paint disagree with SSR.
 */
export function useWalletAvailable(): boolean {
  return useSyncExternalStore(subscribeAvailability, isWalletAvailable, () => false);
}
