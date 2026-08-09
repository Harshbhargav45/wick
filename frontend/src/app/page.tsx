import Link from 'next/link';
import { IgnitionMark } from '@/components/IgnitionMark';

export default function LandingPage() {
  return (
    <main className="min-h-screen bg-background text-foreground">
      {/* top rail */}
      <nav className="mx-auto flex max-w-6xl items-center justify-between px-6 py-5">
        <div className="flex items-center gap-2">
          <IgnitionMark />
          <span className="font-serif text-lg font-semibold tracking-[0.12em]">WICK</span>
        </div>
        <Link
          href="/console"
          className="rounded-md border border-border px-4 py-2 text-sm font-medium text-foreground transition-colors hover:bg-muted"
        >
          Enter console
        </Link>
      </nav>

      {/* hero */}
      <section className="bg-brand-bg px-6">
        <div className="mx-auto max-w-6xl pb-24 pt-16 text-center sm:pt-24">
          <div className="mx-auto mb-6 inline-flex items-center gap-2 rounded-full border border-border bg-card px-3.5 py-1.5 text-xs font-medium text-muted-foreground">
            <span className="inline-block h-1.5 w-1.5 rounded-full bg-success" />
            on-solana · acting within the slot
          </div>

          <h1 className="mx-auto max-w-3xl font-serif text-4xl font-semibold leading-tight tracking-tight text-foreground sm:text-5xl lg:text-6xl">
            Your position has a guard that fires before the liquidator does.
          </h1>

          <p className="mx-auto mt-6 max-w-xl text-base leading-7 text-muted-foreground sm:text-lg">
            Wick watches a leveraged perps position on Solana, reads its health on
            every slot, and acts at the threshold — autonomous on Drift, co-signed on
            Jupiter. You hold the keys. Wick holds the trigger.
          </p>

          <div className="mt-10 flex flex-col items-center justify-center gap-3 sm:flex-row">
            <Link
              href="/console"
              className="inline-flex items-center gap-2 rounded-md bg-primary px-6 py-3 text-base font-semibold text-primary-foreground transition-colors hover:brightness-110"
            >
              Enter the console
            </Link>
            <Link
              href="#how"
              className="inline-flex items-center rounded-md border border-border px-6 py-3 text-base font-medium text-foreground transition-colors hover:bg-muted"
            >
              How it fires
            </Link>
          </div>

          {/* trust strip */}
          <div className="mx-auto mt-16 flex max-w-2xl flex-wrap items-center justify-center gap-x-8 gap-y-3 border-t border-border pt-8">
            <span className="text-xs uppercase tracking-wider text-muted-foreground">
              with
            </span>
            {['Drift', 'Jupiter', 'Pyth', 'Solana'].map((word) => (
              <span
                key={word}
                className="font-mono text-sm text-secondary-foreground/70"
              >
                {word}
              </span>
            ))}
          </div>
        </div>
      </section>

      {/* how it fires */}
      <section id="how" className="px-6">
        <div className="mx-auto max-w-6xl py-20">
          <div className="max-w-2xl">
            <h2 className="font-serif text-3xl font-semibold tracking-tight text-foreground sm:text-4xl">
              The guard, explained
            </h2>
            <p className="mt-4 text-base leading-7 text-muted-foreground">
              A delegated guard reads the position&apos;s health each slot. When it dips, the
              guard responds on any of three authority profiles — depending on the venue.
            </p>
          </div>

          <div className="mt-12 grid gap-4 sm:grid-cols-3">
            {[
              {
                title: 'health',
                body: 'Health factor is recomputed per slot from live mark and collateral. Trigger buffer sits above the liquidation threshold — 1.15, so there is room to move.',
              },
              {
                title: 'trigger',
                body: 'At the threshold the guard reduces. It targets the position adjustment, signs the order, and lands it — without waiting for you, on delegated venues.',
              },
              {
                title: 'proof',
                body: 'Every action is signed, settled, and logged. The console shows the last slot checked and the exact latency of each dispatch. No guesswork.',
              },
            ].map((card) => (
              <div
                key={card.title}
                className="rounded-lg border border-border bg-card p-6"
              >
                <div className="mb-3 font-mono text-xs uppercase tracking-wider text-muted-foreground">
                  {card.title}
                </div>
                <p className="text-sm leading-6 text-muted-foreground">{card.body}</p>
              </div>
            ))}
          </div>
        </div>
      </section>

      {/* numbers / proof */}
      <section className="border-t border-border px-6">
        <div className="mx-auto max-w-6xl py-20">
          <div className="max-w-2xl">
            <h2 className="font-serif text-3xl font-semibold tracking-tight text-foreground sm:text-4xl">
              Latency is the product
            </h2>
            <p className="mt-4 text-base leading-7 text-muted-foreground">
              Dispatch is measured end-to-end against the L1 slot. Numbers below are from a
              recorded sample of autonomous dispatch.
            </p>
          </div>

          <div className="mt-12 grid gap-4 sm:grid-cols-4">
            {[
              { label: 'p50', value: '375µs', note: 'vs ~400ms slot' },
              { label: 'p99', value: '564µs', note: 'worst recorded' },
              { label: 'samples', value: '300', note: 'on-chain sample' },
              { label: 'buffer', value: '1.15', note: 'trigger under liq' },
            ].map((stat) => (
              <div
                key={stat.label}
                className="rounded-lg border border-border bg-card p-6"
              >
                <div className="text-xs uppercase tracking-wider text-muted-foreground">
                  {stat.label}
                </div>
                <div className="mt-2 font-mono text-3xl tracking-tight text-foreground">
                  {stat.value}
                </div>
                <div className="mt-1 text-xs text-muted-foreground">{stat.note}</div>
              </div>
            ))}
          </div>
        </div>
      </section>

      {/* footer */}
      <footer className="border-t border-border px-6">
        <div className="mx-auto flex max-w-6xl flex-col items-center justify-between gap-4 py-8 sm:flex-row">
          <div className="flex items-center gap-2">
            <IgnitionMark className="opacity-70" />
            <span className="font-mono text-xs text-muted-foreground">
              wick · confidential protector engine
            </span>
          </div>
          <span className="font-mono text-xs text-muted-foreground">
            devnet · drift delegated · jupiter co-signed
          </span>
        </div>
      </footer>
    </main>
  );
}