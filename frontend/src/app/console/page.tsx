'use client';

import { useState } from 'react';
import Link from 'next/link';
import { PulseStrip } from '@/components/PulseStrip';
import { PositionHeader } from '@/components/PositionHeader';
import { HealthCard } from '@/components/HealthCard';
import { PositionStats } from '@/components/PositionStats';
import { LatencyChart } from '@/components/LatencyChart';
import { LiveMeta } from '@/components/LiveMeta';
import { ActivityLog, ActivityEvent } from '@/components/ActivityLog';
import { ConfirmationSheet } from '@/components/ConfirmationSheet';
import { ToastLayer, ToastMessage } from '@/components/ToastLayer';
import { useGuardAccount } from '@/hooks/useGuardAccount';
import styles from '@/components/Console.module.css';

const INITIAL_EVENTS: ActivityEvent[] = [
  { id: '1', type: 'healthy', message: 'Tick accepted — healthy', timeAgo: '12s ago' },
  { id: '2', type: 'healthy', message: 'Tick accepted — healthy', timeAgo: '13s ago' },
  { id: '3', type: 'action', message: 'Partial close fired — 18% (auto)', timeAgo: '2m ago' },
  { id: '4', type: 'healthy', message: 'Tick accepted — healthy', timeAgo: '2m ago' },
];

export default function ConsolePage() {
  const { state: liveState, error } = useGuardAccount();

  const [acknowledgedSlot, setAcknowledgedSlot] = useState<number | null>(null);
  const [toasts, setToasts] = useState<ToastMessage[]>([]);

  const [mountTime] = useState(() => Date.now());
  const venue = liveState?.venue || 'Drift · delegated';
  const isCosigned = liveState?.venue === 'Jupiter';
  const isPending = Boolean(liveState?.isPending);
  const authorityState: 'autonomous' | 'cosigned-idle' | 'cosigned-pending' = isCosigned
    ? isPending
      ? 'cosigned-pending'
      : 'cosigned-idle'
    : 'autonomous';

  const promptPending = isCosigned && isPending;
  const isSheetOpen = promptPending && liveState?.lastCheckSlot !== acknowledgedSlot;

  const handleConfirm = () => {
    setAcknowledgedSlot(liveState?.lastCheckSlot ?? null);
    setToasts(prev => [...prev, { id: Date.now().toString(), message: 'Confirmation sent' }]);
  };

  const handleDismissToast = (id: string) => {
    setToasts(prev => prev.filter(t => t.id !== id));
  };

  return (
    <main className="min-h-screen bg-background text-foreground">
      {/* app header */}
      <header className="mx-auto flex max-w-6xl items-center justify-between px-6 py-5">
        <Link href="/" className="flex items-center gap-2">
          <span className="font-serif text-lg font-semibold tracking-[0.12em]">WICK</span>
          <span className="rounded border border-border px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
            console
          </span>
        </Link>
        <div className="flex items-center gap-2">
          <span className="inline-block h-1.5 w-1.5 rounded-full bg-success" />
          <span className="font-mono text-xs text-muted-foreground">{venue}</span>
        </div>
      </header>

      {/* two-column console */}
      <div className="mx-auto grid max-w-6xl gap-6 px-6 pb-20 lg:grid-cols-[300px_1fr]">
        {/* left rail */}
        <aside className="order-2 lg:order-1">
          <div className="space-y-4 lg:sticky lg:top-6">
            <div className="rounded-lg border border-border bg-card p-4">
              <div className="text-xs uppercase tracking-wider text-muted-foreground">
                Authority
              </div>
              <div className="mt-2 text-sm font-medium text-foreground">
                {isCosigned ? 'Co-signed · Jupiter' : 'Autonomous · Drift'}
              </div>
              <div className="mt-1 text-xs text-muted-foreground">
                {liveState?.isPending
                  ? 'Awaiting your confirmation to land.'
                  : liveState?.isDegraded
                    ? 'Guard degraded — driver skipped.'
                    : 'Delegated guard holds the trigger.'}
              </div>
              {error && (
                <div className="mt-2 text-xs text-destructive">Live sync: {error}</div>
              )}
            </div>

            <div className="rounded-lg border border-border bg-card">
              <div className="border-b border-border p-4">
                <div className="text-xs uppercase tracking-wider text-muted-foreground">Venue</div>
                <div className="mt-2 text-sm font-medium text-foreground">{venue}</div>
              </div>
              <div className="p-4">
                <div className="text-xs uppercase tracking-wider text-muted-foreground">Guard policy</div>
                <div className="mt-3 space-y-2 text-xs">
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Trigger buffer</span>
                    <span className="font-mono text-foreground">1.15</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Liquidation</span>
                    <span className="font-mono text-foreground">1.00</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Max reduce</span>
                    <span className="font-mono text-foreground">18%</span>
                  </div>
                </div>
              </div>
            </div>

            <div className="rounded-lg border border-border bg-card p-4">
              <div className="text-xs uppercase tracking-wider text-muted-foreground">
                What the guard does
              </div>
              <p className="mt-2 text-xs leading-5 text-muted-foreground">
                Reads health per slot. At the trigger it reduces the position so the
                liquidator loses the margin. Every dispatch is signed, logged, and
                provable — the console shows the latency of each one.
              </p>
            </div>
          </div>
        </aside>

        {/* right column — live console */}
        <section className="order-1 lg:order-2">
          <PulseStrip
            mode={liveState?.isDegraded ? 'boot' : 'live'}
            lastEventSeverity={liveState?.healthFactor && liveState.healthFactor <= 1.15 ? 'warn' : 'healthy'}
          />

          <PositionHeader
            venueLabel={venue}
            venueTitle="SOL-PERP · LONG"
            authorityState={authorityState}
          />

          <div className="grid gap-4 md:grid-cols-2">
            <HealthCard healthFactor={liveState?.healthFactor || 1.34} liquidationThreshold={1.0} triggerBuffer={1.15} />
            <PositionStats />
          </div>

          <div className="mt-4">
            <LatencyChart />
          </div>

          <div className="mt-4">
            <LiveMeta
              lastCheckTime={mountTime}
              slot={liveState?.lastCheckSlot ? liveState.lastCheckSlot.toString() : '341,208,119'}
            />
          </div>

          <div className={styles.divider} />

          <ActivityLog events={INITIAL_EVENTS} />
        </section>
      </div>

      <ConfirmationSheet
        isOpen={isSheetOpen}
        onConfirm={handleConfirm}
        onDismiss={() => setAcknowledgedSlot(liveState?.lastCheckSlot ?? null)}
        triggerPrice="$182.50"
        positionFraction="18%"
      />

      <ToastLayer toasts={toasts} onDismiss={handleDismissToast} />
    </main>
  );
}