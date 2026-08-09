'use client';

import { formatQty, formatUsd } from '@/lib/guard-health';
import type { Health } from '@/lib/guard-health';
import type { GuardState } from '@/lib/guard-layout';
import { cn } from '@/lib/utils';

function Row({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: 'positive' | 'negative';
}) {
  return (
    <div className="flex items-baseline justify-between gap-4 border-b border-border/60 py-2.5 last:border-b-0">
      <dt className="text-[12.5px] text-muted-foreground">{label}</dt>
      <dd
        className={cn(
          'font-mono text-[12.5px] tabular-nums',
          tone === 'positive' && 'text-healthy',
          tone === 'negative' && 'text-risk',
          !tone && 'text-foreground',
        )}
      >
        {value}
      </dd>
    </div>
  );
}

export function PositionPanel({ state, health }: { state: GuardState; health: Health }) {
  const isLong = state.size > 0n;
  const pnl = health.pnl;
  const pnlPct =
    state.collateral === 0n ? null : (Number(pnl) / Number(state.collateral)) * 100;

  return (
    <div className="rounded-xl border border-border bg-surface/40 p-5">
      <div className="flex items-center justify-between">
        <span className="font-mono text-[11px] tracking-[0.24em] text-muted-foreground">
          POSITION
        </span>
        <span
          className={cn(
            'rounded border px-1.5 py-0.5 font-mono text-[10px] tracking-[0.16em]',
            isLong ? 'border-healthy/40 text-healthy' : 'border-risk/40 text-risk',
          )}
        >
          {isLong ? 'LONG' : 'SHORT'}
        </span>
      </div>

      <dl className="mt-3">
        <Row label="Collateral" value={formatUsd(state.collateral)} />
        <Row label="Size" value={formatQty(state.size, 4)} />
        <Row label="Entry" value={formatUsd(state.entry, 4)} />
        <Row label="Mark" value={formatUsd(state.currentPrice, 4)} />
        <Row
          label="Unrealized PnL"
          value={
            pnlPct === null
              ? formatUsd(pnl)
              : `${formatUsd(pnl)} (${pnlPct >= 0 ? '+' : ''}${pnlPct.toFixed(2)}%)`
          }
          tone={pnl > 0n ? 'positive' : pnl < 0n ? 'negative' : undefined}
        />
      </dl>
    </div>
  );
}
