'use client';

import { useEffect, useRef, useState } from 'react';
import { describeActionText } from '@/lib/guard-events';
import type { GuardSnapshot } from './useGuardAccount';

export type GuardEventKind = 'healthy' | 'action' | 'warn' | 'risk';

export interface GuardEvent {
  id: string;
  kind: GuardEventKind;
  message: string;
  slot: string;
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
    const slot = snapshot.state.lastCheckSlot.toString();

    if (!before) {
      next.push({
        kind: snapshot.state.degraded ? 'risk' : 'healthy',
        message: `Attached to guard at nonce ${snapshot.state.nonce}`,
        slot,
      });
    } else {
      if (before.state.nonce !== snapshot.state.nonce) {
        next.push({
          kind: 'action',
          message: `Nonce committed — ${before.state.nonce} → ${snapshot.state.nonce}`,
          slot,
        });
      }

      const beforeAction = describeActionText(before.state.pending);
      const afterAction = describeActionText(snapshot.state.pending);
      if (beforeAction !== afterAction && snapshot.state.pending) {
        next.push({
          kind: snapshot.state.pending.kind === 'EscalateManualReview' ? 'risk' : 'warn',
          message: `Action selected — ${afterAction}`,
          slot,
        });
      }
      if (before.state.pending && !snapshot.state.pending) {
        next.push({ kind: 'healthy', message: 'Pending action cleared', slot });
      }

      if (!before.state.degraded && snapshot.state.degraded) {
        next.push({
          kind: 'risk',
          message: `Degraded — ${snapshot.state.staleStreak} consecutive stale ticks`,
          slot,
        });
      }
      if (before.state.degraded && !snapshot.state.degraded) {
        next.push({ kind: 'healthy', message: 'Fresh tick — degraded cleared', slot });
      }

      if (!before.health.breachingBuffer && snapshot.health.breachingBuffer) {
        next.push({ kind: 'warn', message: 'Trigger buffer breached', slot });
      }
      if (!before.health.liquidatable && snapshot.health.liquidatable) {
        next.push({ kind: 'risk', message: 'Below maintenance margin', slot });
      }

      if (
        before.state.lastCheckSlot !== snapshot.state.lastCheckSlot &&
        next.length === 0
      ) {
        next.push({ kind: 'healthy', message: 'Tick accepted — no action needed', slot });
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
