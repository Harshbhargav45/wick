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

  return (
    <div className="rounded-xl border border-border bg-surface/40 p-5">
      <div className="flex items-baseline justify-between">
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
          'mt-3 font-serif text-5xl tabular-nums',
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

      <div className="relative mt-2 h-4 font-mono text-[10px] text-muted-foreground">
        <span className="absolute -translate-x-1/2" style={{ left: `${pct(1)}%` }}>
          liq 1.00
        </span>
        <span className="absolute -translate-x-1/2" style={{ left: `${pct(triggerFactor)}%` }}>
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
