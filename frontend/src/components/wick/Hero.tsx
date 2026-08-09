import Link from 'next/link';
import { ArrowRight } from 'lucide-react';
import { HeroTerminal } from './HeroTerminal';

const HEADLINE = ['Your position has a guard', 'that fires before the', 'liquidator does.'];

export function Hero() {
  return (
    <section className="relative overflow-hidden pt-32 pb-20 sm:pt-40 sm:pb-28">
      <div aria-hidden="true" className="pointer-events-none absolute inset-0">
        <div className="absolute inset-0 wick-grid opacity-[0.35] [mask-image:radial-gradient(70%_55%_at_50%_0%,black,transparent)]" />
        <div className="absolute inset-0 wick-radial" />
        <svg className="absolute inset-x-0 top-24 h-64 w-full opacity-30" viewBox="0 0 1200 260" fill="none">
          <path
            d="M0 200 L280 200 L340 140 L620 140 L700 60 L1200 60"
            stroke="var(--border-strong)"
            strokeWidth="1"
            strokeDasharray="4 6"
          />
          <path
            d="M0 240 L420 240 L500 180 L900 180 L980 110 L1200 110"
            stroke="var(--border)"
            strokeWidth="1"
          />
        </svg>
      </div>

      <div className="relative mx-auto max-w-6xl px-4 sm:px-6">
        <div className="mx-auto max-w-3xl text-center">
          <span className="inline-flex items-center gap-2 rounded-full border border-border bg-surface/60 px-3 py-1 font-mono text-[11px] tracking-[0.16em] text-muted-foreground backdrop-blur">
            <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-healthy animate-pulse-dot" />
            on-solana · acting within the slot
          </span>

          <h1 className="mt-8 font-serif text-[clamp(2.5rem,7vw,4.75rem)] leading-[1.04] tracking-tight text-foreground">
            {HEADLINE.map((line, i) => (
              <span key={line} className="block overflow-hidden">
                <span
                  className="block animate-fade-in"
                  style={{ animationDelay: `${120 + i * 130}ms` }}
                >
                  {line}
                </span>
              </span>
            ))}
          </h1>

          <p
            className="mx-auto mt-6 max-w-xl animate-fade-in text-balance text-base leading-relaxed text-muted-foreground sm:text-lg"
            style={{ animationDelay: '560ms' }}
          >
            Wick reads a leveraged perps position on every slot and acts at the threshold —
            autonomous on Drift, co-signed on Jupiter. You hold the keys. Wick holds the trigger.
          </p>

          <div
            className="mt-9 flex animate-fade-in flex-col items-center justify-center gap-3 sm:flex-row"
            style={{ animationDelay: '700ms' }}
          >
            <Link
              href="/console"
              className="group inline-flex w-full items-center justify-center gap-2 rounded-md bg-primary px-5 py-3 text-sm font-semibold text-primary-foreground transition-all hover:brightness-110 active:translate-y-px sm:w-auto"
            >
              Enter the console
              <ArrowRight className="h-4 w-4 transition-transform group-hover:translate-x-0.5" />
            </Link>
            <Link
              href="#mechanism"
              className="inline-flex w-full items-center justify-center rounded-md border border-border-strong px-5 py-3 text-sm font-medium text-foreground transition-all hover:border-primary hover:text-primary active:translate-y-px sm:w-auto"
            >
              How it fires
            </Link>
          </div>
        </div>

        <div className="mx-auto mt-16 max-w-3xl animate-fade-in" style={{ animationDelay: '820ms' }}>
          <HeroTerminal />
        </div>
      </div>
    </section>
  );
}
