import Link from 'next/link';
import { WickMark } from './Logo';
import { Marquee } from './Marquee';
import { PoweredByMagicBlock } from './PoweredByMagicBlock';
import { footerTicker, magicblock } from '@/lib/wick-data';

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
      { label: 'MagicBlock', href: magicblock.href },
      { label: 'Solana', href: 'https://solana.com' },
    ],
  },
  {
    title: 'RESOURCES',
    links: [
      { label: 'GitHub', href: 'https://github.com/Harshbhargav45/wick' },
      { label: 'Solana docs', href: 'https://solana.com/docs' },
      { label: 'Pyth pull oracle', href: 'https://docs.pyth.network' },
      { label: 'MagicBlock docs', href: magicblock.docsHref },
    ],
  },
];

export function Footer() {
  return (
    <footer className="border-t border-border bg-surface/30">
      {/* A second, slower rail running the other way from the one above the
          fold, so the two do not read as the same strip repeated. */}
      <Marquee
        label="Wick properties"
        durationS={64}
        reverse
        className="border-b border-border/70 py-5"
      >
        {footerTicker.map((line) => (
          <span
            key={line}
            className="mr-3 flex shrink-0 items-center gap-3 rounded-md border border-border/70 bg-background/40 px-4 py-2 font-mono text-[11px] tracking-[0.1em] text-muted-foreground"
          >
            <span aria-hidden="true" className="h-1 w-1 rounded-full bg-primary" />
            {line}
          </span>
        ))}
      </Marquee>

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
            <div className="mt-5 flex flex-wrap items-center gap-2">
              <span className="inline-flex items-center gap-2 rounded-md border border-border px-2.5 py-1.5 font-mono text-[11px] text-muted-foreground">
                <span className="h-1.5 w-1.5 rounded-full bg-healthy" aria-hidden="true" />
                devnet · drift delegated · jupiter co-signed
              </span>
            </div>
            <PoweredByMagicBlock variant="feature" className="mt-4" />
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
          <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
            <span>All latency figures are recorded benchmark samples.</span>
            <PoweredByMagicBlock variant="inline" />
          </div>
        </div>
      </div>
    </footer>
  );
}
