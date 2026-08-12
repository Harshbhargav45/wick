'use client';

import { useEffect, useRef, useState } from 'react';
import { describeActionText } from '@/lib/guard-events';
import type { GuardSnapshot } from './useGuardAccount';

export type GuardEventKind = 'healthy' | 'action' | 'warn' | 'risk';

export interface GuardEvent {
  id: string;
  kind: GuardEventKind;
  message: string;
  /**
   * Unix seconds of the guard tick this transition was observed at, from the
   * account's own `last_check_ts` — not the browser's clock, and not a slot,
   * which is what this field used to hold before the layout was corrected.
   */
  tickTs: bigint;
  /** Wall-clock ms when this browser saw it, for the relative "12s ago". */
  at: number;
}

const MAX_EVENTS = 24;

/**
 * An append-only log of transitions this browser session actually observed.
 *
 * It is deliberately not backfilled: the guard account holds current state, not
 * history, so anything before the page loaded is unknown to us and is not
 * invented here.
 */
export function useGuardEvents(snapshot: GuardSnapshot | null) {
  const [events, setEvents] = useState<GuardEvent[]>([]);
  const prev = useRef<GuardSnapshot | null>(null);
  const seq = useRef(0);

  useEffect(() => {
    if (!snapshot) return;
    const before = prev.current;
    prev.current = snapshot;

    const next: Omit<GuardEvent, 'id' | 'at'>[] = [];
    const tickTs = snapshot.state.lastCheckTs;

    if (!before) {
      next.push({
        kind: snapshot.state.degraded ? 'risk' : 'healthy',
        message: `Attached to guard at nonce ${snapshot.state.nonce}`,
        tickTs,
      });
    } else {
      if (before.state.nonce !== snapshot.state.nonce) {
        next.push({
          kind: 'action',
          message: `Nonce committed — ${before.state.nonce} → ${snapshot.state.nonce}`,
          tickTs,
        });
      }

      const beforeAction = describeActionText(before.state.pending);
      const afterAction = describeActionText(snapshot.state.pending);
      if (beforeAction !== afterAction && snapshot.state.pending) {
        next.push({
          kind: snapshot.state.pending.kind === 'EscalateManualReview' ? 'risk' : 'warn',
          message: `Action selected — ${afterAction}`,
          tickTs,
        });
      }
      if (before.state.pending && !snapshot.state.pending) {
        next.push({ kind: 'healthy', message: 'Pending action cleared', tickTs });
      }

      if (!before.state.degraded && snapshot.state.degraded) {
        next.push({
          kind: 'risk',
          message: `Degraded — ${snapshot.state.staleStreak} consecutive stale ticks`,
          tickTs,
        });
      }
      if (before.state.degraded && !snapshot.state.degraded) {
        next.push({ kind: 'healthy', message: 'Fresh tick — degraded cleared', tickTs });
      }

      // §8.8. Worth its own line in the log because it is the one transition
      // that stops the guard acting while every number on screen still reads
      // healthy — nothing else in this feed would show it.
      if (!before.health.diverged && snapshot.health.diverged) {
        next.push({
          kind: 'risk',
          message: 'Venue diverged — the guard will not execute on this position',
          tickTs,
        });
      }
      if (before.health.diverged && !snapshot.health.diverged) {
        next.push({ kind: 'healthy', message: 'Venue reconciled — guard re-armed', tickTs });
      }

      if (!before.health.breachingBuffer && snapshot.health.breachingBuffer) {
        next.push({ kind: 'warn', message: 'Trigger buffer breached', tickTs });
      }
      if (!before.health.liquidatable && snapshot.health.liquidatable) {
        next.push({ kind: 'risk', message: 'Below maintenance margin', tickTs });
      }

      if (before.state.lastCheckTs !== snapshot.state.lastCheckTs && next.length === 0) {
        next.push({ kind: 'healthy', message: 'Tick accepted — no action needed', tickTs });
      }
    }

    if (next.length === 0) return;
    const at = Date.now();
    setEvents((current) =>
      [
        ...next.map((e) => ({ ...e, id: `${at}-${seq.current++}`, at })).reverse(),
        ...current,
      ].slice(0, MAX_EVENTS),
    );
  }, [snapshot]);

  return events;
}
