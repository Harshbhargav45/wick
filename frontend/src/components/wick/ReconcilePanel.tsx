'use client';

import {
  RECONCILE_CONVERGED,
  RECONCILE_DIVERGED,
  RECONCILE_NEVER,
  VENUE_DRIFT,
  type GuardState,
} from '@/lib/guard-layout';
import { formatQty, formatUsd } from '@/lib/guard-health';
import { cn } from '@/lib/utils';

/**
 * §8.8 — what the venue's own account said, and whether the guard agreed.
 *
 * Read-only on purpose. `ReconcileVenue` is permissionless and takes the venue
 * position account, which the cranker already holds and refreshes every tick;
 * putting a button here would offer the owner a slower copy of something already
 * running, and the useful action on a divergence is not "reconcile again" — it
 * is to re-enroll the position, which lives in the owner actions panel.
 */
export function ReconcilePanel({ state, chainTs }: { state: GuardState; chainTs: bigint }) {
  // Only the autonomous tier has a position account the program can decode, so
  // for anything else this panel would be reporting on a check that never runs.
  if (state.venue !== VENUE_DRIFT) return null;

  const { reconcile } = state;
  const diverged = reconcile.status === RECONCILE_DIVERGED;
  const never = reconcile.status === RECONCILE_NEVER;

  const ageSecs = never ? null : chainTs - reconcile.ts;

  return (
    <div
      className={cn(
        'rounded-xl border bg-surface/40',
        diverged ? 'border-risk/50' : never ? 'border-warning/40' : 'border-border',
      )}
    >
      <div className="flex items-center justify-between border-b border-border px-4 py-3">
        <span className="font-mono text-[11px] tracking-[0.24em] text-muted-foreground">
          VENUE RECONCILIATION
        </span>
        <span
          className={cn(
            'font-mono text-[11px]',
            diverged ? 'text-risk' : never ? 'text-warning' : 'text-healthy',
          )}
        >
          {diverged ? 'DIVERGED' : never ? 'never run' : 'converged'}
        </span>
      </div>

      <div className="px-4 py-4">
        {never ? (
          <p className="text-[12px] leading-relaxed text-muted-foreground">
            The guard&apos;s position has never been checked against the venue&apos;s own account.
            Reconciliation is permissionless and runs from the cranker — until it does, the numbers
            below the health gauge are the guard&apos;s model and nothing has confirmed them.
          </p>
        ) : (
          <>
            {diverged ? (
              <p className="mb-3 text-[12px] leading-relaxed text-risk">
                The venue reports a different position than the guard is watching. Autonomous
                execution is refused while this holds — the guard will not act on numbers it knows
                are wrong. Re-enroll the position to clear it.
              </p>
            ) : null}

            <dl className="space-y-2.5 text-[12px]">
              <Row
                label="Size — guard / venue"
                value={`${formatQty(state.size)} / ${formatQty(reconcile.venueSize)}`}
                tone={diverged ? 'risk' : 'default'}
              />
              <Row
                label="Collateral — guard / venue"
                value={`${formatUsd(state.collateral)} / ${formatUsd(reconcile.venueCollateral)}`}
                tone={diverged ? 'risk' : 'default'}
              />
              <Row
                label="Last checked"
                value={ageSecs === null ? '—' : describeAge(ageSecs)}
                tone={
                  reconcile.status === RECONCILE_CONVERGED && ageSecs !== null && ageSecs > 3_600n
                    ? 'warning'
                    : 'default'
                }
              />
              <Row label="Reconcile nonce" value={reconcile.nonce.toString()} />
            </dl>
          </>
        )}
      </div>
    </div>
  );
}

function Row({
  label,
  value,
  tone = 'default',
}: {
  label: string;
  value: string;
  tone?: 'default' | 'warning' | 'risk';
}) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <dt className="text-muted-foreground">{label}</dt>
      <dd
        className={cn(
          'font-mono tabular-nums',
          tone === 'risk' && 'text-risk',
          tone === 'warning' && 'text-warning',
          tone === 'default' && 'text-foreground',
        )}
      >
        {value}
      </dd>
    </div>
  );
}

/**
 * Age against the chain's clock, not the browser's.
 *
 * A negative age means the account's timestamp is ahead of the clock we read —
 * possible across a failover to a lagging RPC. Reported as "just now" rather
 * than as a negative duration, which would read as a bug in the guard.
 */
function describeAge(secs: bigint): string {
  if (secs <= 0n) return 'just now';
  if (secs < 60n) return `${secs}s ago`;
  if (secs < 3_600n) return `${secs / 60n}m ago`;
  if (secs < 86_400n) return `${secs / 3_600n}h ago`;
  return `${secs / 86_400n}d ago`;
}
