'use client';

import { useState } from 'react';
import { headroom, latencyBuckets, latencyStats } from '@/lib/wick-data';
import { useCountUp, useReveal } from '@/hooks/useWickMotion';
import { LatencyGraph } from './LatencyGraph';
import { Reveal } from './Reveal';
import { cn } from '@/lib/utils';

function Stat({
  label,
  value,
  suffix,
  note,
}: {
  label: string;
  value: number;
  suffix: string;
  note: string;
}) {
  const { ref, visible } = useReveal<HTMLDivElement>(0.4);
  const n = useCountUp(value, visible, 1200);

  return (
    <div
      ref={ref}
      className="group rounded-lg border border-border bg-surface/50 p-5 transition-all duration-300 hover:-translate-y-0.5 hover:border-border-strong"
    >
      <div className="font-mono text-[11px] tracking-[0.2em] text-muted-foreground">{label}</div>
      <div className="mt-3 font-mono text-3xl tabular-nums text-foreground">
        {Math.round(n)}
        <span className="text-primary">{suffix}</span>
      </div>
      <div className="mt-1.5 text-xs text-muted-foreground">{note}</div>
    </div>
  );
}

const STATS = [
  { label: 'P50', value: latencyStats.p50Us, suffix: 'µs', note: `vs ~${latencyStats.slotMs}ms slot` },
  { label: 'P99', value: latencyStats.p99Us, suffix: 'µs', note: 'recorded tail' },
  { label: 'SAMPLES', value: latencyStats.samples, suffix: '', note: 'recorded dispatches' },
  { label: 'HEADROOM', value: headroom, suffix: '×', note: `under the ${latencyStats.targetMs}ms lane` },
];

export function LatencySection() {
  const [tab, setTab] = useState<'dispatch' | 'distribution'>('dispatch');

  return (
    <section id="latency" className="border-t border-border/70 py-24 sm:py-32">
      <div className="mx-auto max-w-6xl px-4 sm:px-6">
        <Reveal className="max-w-2xl">
          <span className="font-mono text-[11px] tracking-[0.24em] text-primary">02 — SPEED</span>
          <h2 className="mt-4 font-serif text-4xl leading-tight text-foreground sm:text-5xl">
            Latency is the product
          </h2>
          <p className="mt-4 text-muted-foreground">
            Dispatch is measured from the tick entering the guard to the venue CPI landing. Every
            number below is read out of a recorded benchmark run — not a live feed, and not a
            projection.
          </p>
        </Reveal>

        <Reveal
          className="mt-10 rounded-xl border border-border bg-surface/40 p-4 sm:p-6"
          delay={80}
        >
          <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 sm:flex sm:justify-between">
            <div className="min-w-0">
              <div className="truncate font-mono text-[11px] tracking-[0.2em] text-muted-foreground">
                AUTONOMOUS DISPATCH LATENCY
              </div>
              <div className="mt-1 font-mono text-xs text-muted-foreground">
                recorded sample · n={latencyStats.samples} · vs L1 slot ~{latencyStats.slotMs}ms
              </div>
            </div>
            <div
              role="tablist"
              aria-label="Latency view"
              className="flex shrink-0 rounded-md border border-border p-0.5"
            >
              {(['dispatch', 'distribution'] as const).map((t) => (
                <button
                  key={t}
                  type="button"
                  role="tab"
                  aria-selected={tab === t}
                  onClick={() => setTab(t)}
                  className={cn(
                    'rounded px-3 py-1.5 font-mono text-[11px] tracking-wider transition-all',
                    tab === t
                      ? 'bg-surface-raised text-foreground'
                      : 'text-muted-foreground hover:text-foreground',
                  )}
                >
                  {t.toUpperCase()}
                </button>
              ))}
            </div>
          </div>

          <div className="mt-6">{tab === 'dispatch' ? <LatencyGraph /> : <Distribution />}</div>

          <div className="mt-4 flex flex-wrap gap-x-6 gap-y-1 font-mono text-[11px] text-muted-foreground">
            <span>min {latencyStats.minUs}µs</span>
            <span>p50 {latencyStats.p50Us}µs</span>
            <span>p99 {latencyStats.p99Us}µs</span>
            <span>max {(latencyStats.maxUs / 1000).toFixed(2)}ms</span>
          </div>
          {latencyStats.note ? (
            <p className="mt-2 font-mono text-[11px] text-muted-foreground/70">
              {latencyStats.note}
            </p>
          ) : null}
        </Reveal>

        <div className="mt-6 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          {STATS.map((s) => (
            <Stat key={s.label} {...s} />
          ))}
        </div>
      </div>
    </section>
  );
}

function Distribution() {
  const { ref, visible } = useReveal<HTMLDivElement>(0.3);
  const buckets = latencyBuckets();

  return (
    <div ref={ref} className="space-y-3 py-2">
      {buckets.map((b, i) => (
        <div key={b.label} className="min-w-0">
          <div className="mb-1.5 flex items-center justify-between font-mono text-[11px] text-muted-foreground">
            <span>{b.label}</span>
            <span className="text-foreground">{b.pct}%</span>
          </div>
          <div className="h-1.5 overflow-hidden rounded-full bg-muted">
            <div
              className="h-full rounded-full bg-primary"
              style={{
                width: visible ? `${b.pct}%` : '0%',
                transition: `width 900ms cubic-bezier(0.22,1,0.36,1) ${i * 90}ms`,
              }}
            />
          </div>
        </div>
      ))}
    </div>
  );
}
