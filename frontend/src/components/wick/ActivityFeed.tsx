'use client';

import { useEffect, useState } from 'react';
import type { GuardEvent } from '@/hooks/useGuardEvents';
import { cn } from '@/lib/utils';

const TONE: Record<GuardEvent['kind'], string> = {
  healthy: 'text-healthy',
  action: 'text-primary',
  warn: 'text-warning',
  risk: 'text-risk',
};

const DOT: Record<GuardEvent['kind'], string> = {
  healthy: 'bg-healthy',
  action: 'bg-primary',
  warn: 'bg-warning',
  risk: 'bg-risk',
};

function ago(at: number, now: number): string {
  const secs = Math.max(0, Math.round((now - at) / 1000));
  if (secs < 60) return `${secs}s ago`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  return `${Math.floor(mins / 60)}h ago`;
}

export function ActivityFeed({ events }: { events: GuardEvent[] }) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);

  return (
    <div className="rounded-xl border border-border bg-surface/40">
      <div className="flex items-center justify-between border-b border-border px-4 py-3">
        <span className="font-mono text-[11px] tracking-[0.24em] text-muted-foreground">
          ACTIVITY
        </span>
        <span className="font-mono text-[10px] text-muted-foreground/70">
          this session only
        </span>
      </div>

      {events.length === 0 ? (
        <p className="px-4 py-6 text-[12.5px] text-muted-foreground">
          Nothing observed yet. The guard account holds current state, not history — this log
          starts when the page attaches and records only transitions it sees.
        </p>
      ) : (
        <ul aria-live="polite" className="divide-y divide-border/60">
          {events.map((event) => (
            <li key={event.id} className="flex items-baseline gap-3 px-4 py-2.5">
              <span
                aria-hidden="true"
                className={cn('mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full', DOT[event.kind])}
              />
              <span className={cn('min-w-0 flex-1 text-[12.5px]', TONE[event.kind])}>
                {event.message}
              </span>
              <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground/70">
                slot {event.slot} · {ago(event.at, now)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
