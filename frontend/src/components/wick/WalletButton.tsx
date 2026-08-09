'use client';

import { useState } from 'react';
import { Wallet } from 'lucide-react';
import { useWallet } from '@/hooks/useWallet';
import { connectWallet, disconnectWallet } from '@/lib/wallet';

export function WalletButton() {
  const { connected, publicKey } = useWallet();
  const [error, setError] = useState<string | null>(null);

  const onClick = async () => {
    setError(null);
    try {
      if (connected) await disconnectWallet();
      else await connectWallet();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const label = publicKey
    ? `${publicKey.toBase58().slice(0, 4)}…${publicKey.toBase58().slice(-4)}`
    : 'connect';

  return (
    <div className="relative">
      <button
        type="button"
        onClick={onClick}
        className="flex items-center gap-2 rounded-md border border-border px-2.5 py-1.5 font-mono text-[11px] text-foreground transition-colors hover:border-primary hover:text-primary"
      >
        <Wallet className="h-3.5 w-3.5" aria-hidden="true" />
        {label}
      </button>
      {error ? (
        <p
          role="alert"
          className="absolute right-0 top-full z-50 mt-2 w-64 rounded-md border border-risk/40 bg-surface px-3 py-2 font-mono text-[10.5px] text-risk"
        >
          {error}
        </p>
      ) : null}
    </div>
  );
}
