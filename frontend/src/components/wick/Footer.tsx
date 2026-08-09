import Link from 'next/link';
import { WickMark } from './Logo';

const COLUMNS = [
  {
    title: 'PRODUCT',
    links: [
      { label: 'Console', href: '/console' },
      { label: 'The guard', href: '/#guard' },
      { label: 'Latency', href: '/#latency' },
      { label: 'Mechanism', href: '/#mechanism' },
    ],
  },
  {
    title: 'PROTOCOLS',
    links: [
      { label: 'Drift', href: 'https://drift.trade' },
      { label: 'Jupiter', href: 'https://jup.ag' },
      { label: 'Pyth', href: 'https://pyth.network' },
      { label: 'Solana', href: 'https://solana.com' },
    ],
  },
  {
    title: 'RESOURCES',
    links: [
      { label: 'GitHub', href: 'https://github.com/Harshbhargav45/wick' },
      { label: 'Solana docs', href: 'https://solana.com/docs' },
      { label: 'Pyth pull oracle', href: 'https://docs.pyth.network' },
    ],
  },
];

export function Footer() {
  return (
    <footer className="border-t border-border bg-surface/30">
      <div className="mx-auto max-w-6xl px-4 py-14 sm:px-6">
        <div className="grid gap-10 md:grid-cols-[1.4fr_repeat(3,1fr)]">
          <div className="min-w-0">
            <div className="flex items-center gap-2.5">
              <WickMark />
              <span className="font-mono text-sm font-semibold tracking-[0.28em]">WICK</span>
            </div>
            <p className="mt-4 max-w-xs text-sm leading-relaxed text-muted-foreground">
              A protector engine for leveraged Solana perps. Reads health every tick, acts at the
              threshold, and commits the nonce only once the action lands.
            </p>
            <div className="mt-5 inline-flex items-center gap-2 rounded-md border border-border px-2.5 py-1.5 font-mono text-[11px] text-muted-foreground">
              <span className="h-1.5 w-1.5 rounded-full bg-healthy" aria-hidden="true" />
              devnet · drift delegated · jupiter co-signed
            </div>
          </div>

          {COLUMNS.map((col) => (
            <div key={col.title} className="min-w-0">
              <h3 className="font-mono text-[11px] tracking-[0.24em] text-muted-foreground/70">
                {col.title}
              </h3>
              <ul className="mt-4 space-y-2.5">
                {col.links.map((l) => {
                  const external = l.href.startsWith('http');
                  return (
                    <li key={l.label}>
                      {external ? (
                        <a
                          href={l.href}
                          target="_blank"
                          rel="noreferrer"
                          className="text-sm text-muted-foreground transition-colors hover:text-foreground"
                        >
                          {l.label}
                        </a>
                      ) : (
                        <Link
                          href={l.href}
                          className="text-sm text-muted-foreground transition-colors hover:text-foreground"
                        >
                          {l.label}
                        </Link>
                      )}
                    </li>
                  );
                })}
              </ul>
            </div>
          ))}
        </div>

        <div className="mt-12 flex flex-col gap-3 border-t border-border pt-6 font-mono text-[11px] text-muted-foreground sm:flex-row sm:items-center sm:justify-between">
          <span>wick · confidential protector engine</span>
          <span>All latency figures are recorded benchmark samples.</span>
        </div>
      </div>
    </footer>
  );
}
