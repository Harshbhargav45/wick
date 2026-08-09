'use client';

import { useSyncExternalStore } from 'react';
import {
  getWalletServerSnapshot,
  getWalletSnapshot,
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
