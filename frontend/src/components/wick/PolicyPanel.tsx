'use client';

import { formatBps, formatUsd } from '@/lib/guard-health';
import type { DailyBudget } from '@/lib/guard-health';
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

export function PolicyPanel({ state, budget }: { state: GuardState; budget: DailyBudget }) {
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
          <Row label="Daily total" value={formatUsd(budget.total)} />
        </dl>

        {/* The daily cap is an accumulator, not a per-action ceiling — showing
            only the limit would repeat the old bug where the dashboard read as
            enforced while nothing tracked consumption. */}
        <div className="mt-4 space-y-2">
          <div className="flex items-baseline justify-between gap-3 text-[12px]">
            <span className="text-muted-foreground">Spent today</span>
            <span
              className={`font-mono tabular-nums ${
                budget.exhausted ? 'text-danger' : 'text-foreground'
              }`}
            >
              {formatUsd(budget.spent)}
            </span>
          </div>
          <div
            className="h-1 w-full overflow-hidden rounded-full bg-border"
            role="progressbar"
            aria-label="Daily action budget consumed"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(budget.used * 100)}
          >
            <div
              className={`h-full rounded-full transition-[width] duration-500 ${
                budget.exhausted ? 'bg-danger' : 'bg-accent'
              }`}
              style={{ width: `${Math.max(budget.used * 100, budget.spent > 0n ? 2 : 0)}%` }}
            />
          </div>
          <p className="font-mono text-[10px] text-muted-foreground/70">
            {budget.exhausted
              ? 'budget exhausted — further actions escalate to manual review'
              : `${formatUsd(budget.remaining)} remaining this epoch`}
          </p>
        </div>
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
