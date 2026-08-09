'use client';

import { formatBps, formatUsd } from '@/lib/guard-health';
import type { GuardState } from '@/lib/guard-layout';
import { VENUE_DRIFT } from '@/lib/guard-layout';

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="font-mono tabular-nums text-foreground">{value}</dd>
    </div>
  );
}

export function PolicyPanel({ state }: { state: GuardState }) {
  const { policy } = state;

  return (
    <div className="rounded-xl border border-border bg-surface/40">
      <div className="border-b border-border px-4 py-3">
        <span className="font-mono text-[11px] tracking-[0.24em] text-muted-foreground">
          GUARD POLICY
        </span>
      </div>
      <dl className="space-y-2.5 px-4 py-4 text-[12px]">
        <Row label="Maintenance margin" value={formatBps(policy.maintenanceBps)} />
        <Row label="Trigger buffer" value={formatBps(policy.triggerBufferBps)} />
        <Row label="Fee assumption" value={formatBps(policy.feeBps)} />
        <Row
          label="Take profit"
          value={policy.takeProfit === null ? 'none' : formatUsd(policy.takeProfit, 4)}
        />
      </dl>
      <div className="border-t border-border px-4 py-4">
        <span className="font-mono text-[10px] tracking-[0.2em] text-muted-foreground/70">
          CAPS (USD)
        </span>
        <dl className="mt-3 space-y-2.5 text-[12px]">
          <Row label="Top-up / action" value={formatUsd(policy.caps.topUpUsdPerAction)} />
          <Row
            label="Partial close / action"
            value={formatUsd(policy.caps.partialCloseUsdPerAction)}
          />
          <Row label="Daily total" value={formatUsd(policy.caps.dailyTotalUsd)} />
        </dl>
      </div>
      {state.venue === VENUE_DRIFT ? (
        <div className="border-t border-border px-4 py-4">
          <span className="font-mono text-[10px] tracking-[0.2em] text-muted-foreground/70">
            DRIFT
          </span>
          <dl className="mt-3 space-y-2.5 text-[12px]">
            <Row label="Perp market index" value={String(state.driftMarketIndex)} />
            <Row label="Sub-account" value={String(state.driftSubaccountId)} />
          </dl>
        </div>
      ) : null}
    </div>
  );
}
