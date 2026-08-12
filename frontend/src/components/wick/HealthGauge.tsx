'use client';

import { formatUsd } from '@/lib/guard-health';
import type { Health } from '@/lib/guard-health';
import { cn } from '@/lib/utils';

const MAX_FACTOR = 2;

function pct(factor: number): number {
  return Math.min(100, Math.max(0, (factor / MAX_FACTOR) * 100));
}

export function HealthGauge({ health }: { health: Health }) {
  const { factor, triggerFactor, liquidatable, breachingBuffer } = health;
  const finite = Number.isFinite(factor);

  const tone = liquidatable ? 'risk' : breachingBuffer ? 'warning' : 'healthy';
  const label = liquidatable
    ? 'below maintenance margin'
    : breachingBuffer
      ? 'trigger buffer breached'
      : 'above the trigger buffer';

  const liqPct = pct(1);
  const triggerPct = pct(triggerFactor);
  // Each label runs ~5rem; ticks closer than this share horizontal space.
  const stacked = Math.abs(triggerPct - liqPct) < 18;

  return (
    <div className="rounded-xl border border-border bg-surface/40 p-5">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <span className="font-mono text-[11px] tracking-[0.24em] text-muted-foreground">
          HEALTH FACTOR
        </span>
        <span
          className={cn(
            'font-mono text-[11px]',
            tone === 'risk' && 'text-risk',
            tone === 'warning' && 'text-warning',
            tone === 'healthy' && 'text-healthy',
          )}
        >
          {label}
        </span>
      </div>

      <div
        className={cn(
          'mt-3 font-serif text-4xl tabular-nums sm:text-5xl',
          tone === 'risk' && 'text-risk',
          tone === 'warning' && 'text-warning',
          tone === 'healthy' && 'text-foreground',
        )}
      >
        {finite ? factor.toFixed(2) : '—'}
      </div>

      <div className="relative mt-6 h-2 overflow-hidden rounded-full bg-muted">
        <div
          className={cn(
            'h-full rounded-full transition-[width] duration-700 ease-out',
            tone === 'risk' && 'bg-risk',
            tone === 'warning' && 'bg-warning',
            tone === 'healthy' && 'bg-healthy',
          )}
          style={{ width: `${finite ? pct(factor) : 0}%` }}
        />
        <span
          aria-hidden="true"
          className="absolute top-0 h-full w-px bg-risk/70"
          style={{ left: `${pct(1)}%` }}
        />
        <span
          aria-hidden="true"
          className="absolute top-0 h-full w-px bg-border-strong"
          style={{ left: `${pct(triggerFactor)}%` }}
        />
      </div>

      {/* Both labels are absolutely positioned over their ticks, so two things can
          go wrong on a narrow card: a label at the far end spills past the edge,
          and — at the default 15% buffer the ticks sit only 7.5% apart — the two
          labels land on top of each other. Clamp the anchors, and drop `trigger`
          to its own row when the gap is too small to fit both. */}
      <div className={cn('relative mt-2 font-mono text-[10px] text-muted-foreground', stacked ? 'h-7' : 'h-4')}>
        <span
          className="absolute top-0 -translate-x-1/2 whitespace-nowrap"
          style={{ left: `clamp(2.5rem, ${liqPct}%, calc(100% - 2.5rem))` }}
        >
          liq 1.00
        </span>
        <span
          className={cn('absolute -translate-x-1/2 whitespace-nowrap', stacked ? 'top-3.5' : 'top-0')}
          style={{ left: `clamp(3.25rem, ${triggerPct}%, calc(100% - 3.25rem))` }}
        >
          trigger {triggerFactor.toFixed(2)}
        </span>
      </div>

      <dl className="mt-5 grid grid-cols-2 gap-x-4 gap-y-2.5 border-t border-border pt-4 font-mono text-[11.5px]">
        <div className="flex justify-between">
          <dt className="text-muted-foreground">equity</dt>
          <dd className="text-foreground">{formatUsd(health.equity)}</dd>
        </div>
        <div className="flex justify-between">
          <dt className="text-muted-foreground">margin req</dt>
          <dd className="text-foreground">{formatUsd(health.marginRequired)}</dd>
        </div>
        <div className="flex justify-between">
          <dt className="text-muted-foreground">buffer target</dt>
          <dd className="text-foreground">{formatUsd(health.triggerTarget)}</dd>
        </div>
        <div className="flex justify-between">
          <dt className="text-muted-foreground">notional</dt>
          <dd className="text-foreground">{formatUsd(health.notional)}</dd>
        </div>
      </dl>
    </div>
  );
}
