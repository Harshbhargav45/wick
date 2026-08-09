'use client';

import type { Action } from '@/lib/guard-layout';
import { describeActionText } from '@/lib/guard-events';
import { cn } from '@/lib/utils';

export function ActionPanel({
  action,
  awaitingConfirmation,
  degraded,
  staleStreak,
  nonce,
  pendingIxNonce,
}: {
  action: Action | null;
  awaitingConfirmation: boolean;
  degraded: boolean;
  staleStreak: number;
  nonce: bigint;
  pendingIxNonce: bigint | null;
}) {
  const escalated = action?.kind === 'EscalateManualReview';
  const tone = degraded || escalated ? 'risk' : action ? 'warning' : 'healthy';

  return (
    <div
      className={cn(
        'rounded-xl border bg-surface/40 p-5',
        tone === 'risk' && 'border-risk/50',
        tone === 'warning' && 'border-warning/50',
        tone === 'healthy' && 'border-border',
      )}
    >
      <div className="flex items-center justify-between">
        <span className="font-mono text-[11px] tracking-[0.24em] text-muted-foreground">
          GUARD STATE
        </span>
        <span
          className={cn(
            'flex items-center gap-2 font-mono text-[11px]',
            tone === 'risk' && 'text-risk',
            tone === 'warning' && 'text-warning',
            tone === 'healthy' && 'text-healthy',
          )}
        >
          <span
            aria-hidden="true"
            className={cn(
              'h-1.5 w-1.5 rounded-full',
              tone === 'risk' && 'bg-risk',
              tone === 'warning' && 'bg-warning',
              tone === 'healthy' && 'bg-healthy animate-pulse-dot',
            )}
          />
          {degraded ? 'degraded' : action ? 'action pending' : 'watching'}
        </span>
      </div>

      <p className="mt-4 text-sm leading-relaxed text-muted-foreground">
        {degraded ? (
          <>
            The guard has seen {staleStreak} consecutive stale ticks and has flipped to degraded.
            It will not dispatch on dead price data — a fresh tick clears both the streak and the
            flag.
          </>
        ) : escalated ? (
          <>
            No action inside the configured caps restores the buffer, so the guard escalated rather
            than silently no-oping. This needs a manual decision.
          </>
        ) : action ? (
          <>
            {describeActionText(action)}.{' '}
            {awaitingConfirmation
              ? 'The guard has built the instruction and is holding it — the nonce advances only when you co-sign.'
              : 'Selected on the last tick; the nonce commits once the venue action lands.'}
          </>
        ) : (
          <>
            Health is above the trigger buffer, so the guard is reading ticks and doing nothing. A
            tick that changes nothing is the expected outcome.
          </>
        )}
      </p>

      <dl className="mt-5 grid grid-cols-2 gap-x-4 gap-y-2 border-t border-border pt-4 font-mono text-[11.5px]">
        <div className="flex justify-between">
          <dt className="text-muted-foreground">nonce</dt>
          <dd className="text-foreground">{nonce.toString()}</dd>
        </div>
        <div className="flex justify-between">
          <dt className="text-muted-foreground">stale streak</dt>
          <dd className={cn(staleStreak > 0 ? 'text-warning' : 'text-foreground')}>
            {staleStreak}
          </dd>
        </div>
        {pendingIxNonce !== null ? (
          <div className="col-span-2 flex justify-between">
            <dt className="text-muted-foreground">awaiting co-sign at nonce</dt>
            <dd className="text-warning">{pendingIxNonce.toString()}</dd>
          </div>
        ) : null}
      </dl>
    </div>
  );
}
