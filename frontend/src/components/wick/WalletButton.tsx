'use client';

import { useState } from 'react';
import { ExternalLink, Wallet } from 'lucide-react';
import { useWallet, useWalletAvailable } from '@/hooks/useWallet';
import { connectWallet, disconnectWallet } from '@/lib/wallet';

const WALLETS = [
  { label: 'Phantom', href: 'https://phantom.app/' },
  { label: 'Solflare', href: 'https://solflare.com/' },
];

export function WalletButton() {
  const { connected, publicKey } = useWallet();
  const available = useWalletAvailable();
  const [error, setError] = useState<string | null>(null);
  const [showInstall, setShowInstall] = useState(false);

  const onClick = async () => {
    setError(null);
    // Nothing to connect to. Say that up front with somewhere to go, instead of
    // letting the click fail with a generic "no wallet found".
    if (!connected && !available) {
      setShowInstall((v) => !v);
      return;
    }
    setShowInstall(false);
    try {
      if (connected) await disconnectWallet();
      else await connectWallet();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const label = publicKey
    ? `${publicKey.toBase58().slice(0, 4)}…${publicKey.toBase58().slice(-4)}`
    : !available
      ? 'install wallet'
      : 'connect';

  const panelOpen = showInstall || error !== null;

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
      {panelOpen ? (
        <div
          role={error ? 'alert' : 'dialog'}
          aria-label={error ? undefined : 'Install a Solana wallet'}
          className="absolute right-0 top-full z-50 mt-2 w-64 rounded-md border border-border bg-surface px-3 py-2.5 font-mono text-[10.5px] shadow-lg"
        >
          {error ? (
            <p className="text-risk">{error}</p>
          ) : (
            <>
              <p className="text-muted-foreground">
                No Solana wallet detected. Install one, then reload this page:
              </p>
              <ul className="mt-2 space-y-1.5">
                {WALLETS.map((w) => (
                  <li key={w.label}>
                    <a
                      href={w.href}
                      target="_blank"
                      rel="noreferrer"
                      className="inline-flex items-center gap-1.5 text-foreground transition-colors hover:text-primary"
                    >
                      {w.label}
                      <ExternalLink className="h-3 w-3" aria-hidden="true" />
                    </a>
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      ) : null}
    </div>
  );
}
