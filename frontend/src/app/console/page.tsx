'use client';

import Link from 'next/link';
import { ArrowLeft, RefreshCw } from 'lucide-react';
import { useGuardAccount } from '@/hooks/useGuardAccount';
import { useGuardEvents } from '@/hooks/useGuardEvents';
import { useGuardActions } from '@/hooks/useGuardActions';
import { useWallet } from '@/hooks/useWallet';
import { WickMark } from '@/components/wick/Logo';
import { WalletButton } from '@/components/wick/WalletButton';
import { HealthGauge } from '@/components/wick/HealthGauge';
import { PositionPanel } from '@/components/wick/PositionPanel';
import { PolicyPanel } from '@/components/wick/PolicyPanel';
import { ActionPanel } from '@/components/wick/ActionPanel';
import { ActivityFeed } from '@/components/wick/ActivityFeed';
import { LatencyGraph } from '@/components/wick/LatencyGraph';
import { PoweredByMagicBlock } from '@/components/wick/PoweredByMagicBlock';
import { latencyStats } from '@/lib/wick-data';
import { cn } from '@/lib/utils';

export default function ConsolePage() {
  const { snapshot, status, error, routeConfig, writesBlocked, refresh, rpc, programId } =
    useGuardAccount();
  const events = useGuardEvents(snapshot);
  const { publicKey } = useWallet();
  const { tx, canSend, confirm } = useGuardActions({
    programId,
    guardAddress: snapshot?.address ?? null,
    owner: publicKey,
    onDone: refresh,
  });

  return (
    <div className="min-h-screen bg-background text-foreground">
      <header className="sticky top-0 z-40 border-b border-border bg-background/80 backdrop-blur">
        <div className="mx-auto flex max-w-6xl items-center justify-between gap-2 px-4 py-3 sm:gap-4 sm:px-6 sm:py-3.5">
          <div className="flex min-w-0 items-center gap-2 sm:gap-3">
            <Link href="/" className="flex shrink-0 items-center gap-2.5">
              <WickMark />
              <span className="font-mono text-sm font-semibold tracking-[0.28em]">WICK</span>
            </Link>
            {/* The badge is pure decoration next to a logo that already says where
                you are — first thing to go when the row runs out of room. */}
            <span className="hidden rounded border border-border px-1.5 py-0.5 font-mono text-[10px] tracking-[0.16em] text-muted-foreground sm:inline">
              CONSOLE
            </span>
          </div>

          <div className="flex shrink-0 items-center gap-1.5 sm:gap-3">
            <StatusPill status={status} snapshot={snapshot} />
            <WalletButton />
            <button
              type="button"
              onClick={refresh}
              aria-label="Refresh guard state"
              className="-mr-1.5 grid h-11 w-11 place-items-center rounded text-muted-foreground transition-colors hover:text-foreground sm:h-9 sm:w-9"
            >
              <RefreshCw className="h-4 w-4" />
            </button>
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-6xl px-4 py-8 sm:px-6">
        {routeConfig.paused ? (
          <p className="mb-6 rounded-md border border-risk/40 bg-risk/5 px-3 py-2 font-mono text-[11.5px] text-risk">
            Program paused — the RouteConfig kill-switch is on. Every state-mutating instruction
            rejects until the route authority resumes it.
          </p>
        ) : !routeConfig.exists ? (
          <p className="mb-6 rounded-md border border-warning/40 bg-warning/5 px-3 py-2 font-mono text-[11.5px] text-warning">
            RouteConfig is not initialized. The program checks it on every state-mutating
            instruction, so writes will fail until it exists — run{' '}
            <span className="text-foreground">node src/init.mjs</span> in cranker/.
          </p>
        ) : null}
        {status !== 'ready' || !snapshot ? (
          <EmptyState status={status} error={error} programId={programId} />
        ) : (
          <>
            <div className="flex flex-wrap items-end justify-between gap-4">
              <div className="min-w-0">
                <h1 className="font-serif text-3xl text-foreground">{snapshot.venueLabel}</h1>
                <p className="mt-1 font-mono text-[11px] text-muted-foreground">
                  guard {snapshot.address.slice(0, 6)}…{snapshot.address.slice(-6)} · last check
                  slot {snapshot.state.lastCheckSlot.toString()}
                </p>
              </div>
              <span className="rounded-md border border-border px-2.5 py-1.5 font-mono text-[11px] text-muted-foreground">
                {snapshot.isCoSigned ? 'co-signed · guard builds only' : 'autonomous · guard signs'}
              </span>
            </div>

            {error ? (
              <p className="mt-4 rounded-md border border-warning/40 bg-warning/5 px-3 py-2 font-mono text-[11.5px] text-warning">
                Live sync degraded — showing the last good read. {error}
              </p>
            ) : null}

            <div className="mt-6 grid gap-4 lg:grid-cols-[1fr_320px]">
              <div className="space-y-4">
                <HealthGauge health={snapshot.health} />
                <ActionPanel
                  action={snapshot.state.pending}
                  awaitingConfirmation={snapshot.awaitingConfirmation}
                  degraded={snapshot.state.degraded}
                  staleStreak={snapshot.state.staleStreak}
                  nonce={snapshot.state.nonce}
                  pendingIxNonce={snapshot.state.pendingIxNonce}
                  canConfirm={canSend && snapshot.isOwner && !writesBlocked}
                  onConfirm={confirm}
                  tx={tx}
                />

                <div className="rounded-xl border border-border bg-surface/40 p-5">
                  <div className="flex flex-wrap items-baseline justify-between gap-2">
                    <span className="font-mono text-[11px] tracking-[0.24em] text-muted-foreground">
                      DISPATCH LATENCY
                    </span>
                    <span className="font-mono text-[11px] text-muted-foreground">
                      recorded bench · n={latencyStats.samples} · p50 {latencyStats.p50Us}µs
                    </span>
                  </div>
                  <div className="mt-4">
                    <LatencyGraph />
                  </div>
                </div>

                <ActivityFeed events={events} />
              </div>

              <div className="space-y-4">
                <PositionPanel state={snapshot.state} health={snapshot.health} />
                <PolicyPanel state={snapshot.state} budget={snapshot.budget} />
              </div>
            </div>
          </>
        )}
      </main>

      <footer className="mx-auto mt-4 flex max-w-6xl flex-wrap items-center justify-between gap-3 border-t border-border px-4 py-6 font-mono text-[11px] text-muted-foreground sm:px-6">
        <span>guard state is polled from the configured RPC every 5s</span>
        <PoweredByMagicBlock />
      </footer>
    </div>
  );
}

function StatusPill({
  status,
  snapshot,
}: {
  status: ReturnType<typeof useGuardAccount>['status'];
  snapshot: ReturnType<typeof useGuardAccount>['snapshot'];
}) {
  const degraded = snapshot?.state.degraded ?? false;
  const tone =
    status === 'ready' ? (degraded ? 'risk' : 'healthy') : status === 'loading' ? 'muted' : 'warning';
  const label =
    status === 'ready'
      ? degraded
        ? 'degraded'
        : 'live'
      : status === 'loading'
        ? 'connecting'
        : status === 'empty'
          ? 'no guard'
          : status === 'config'
            ? 'unconfigured'
            : 'error';

  return (
    <span className="flex items-center gap-2 font-mono text-[11px] text-muted-foreground">
      <span
        aria-hidden="true"
        className={cn(
          'h-1.5 w-1.5 shrink-0 rounded-full',
          tone === 'healthy' && 'bg-healthy animate-pulse-dot',
          tone === 'risk' && 'bg-risk',
          tone === 'warning' && 'bg-warning',
          tone === 'muted' && 'bg-border-strong',
        )}
      />
      {/* Below sm the dot carries the signal on its own; the word is still in the
          accessibility tree so the colour is never the only cue. */}
      <span className="sr-only sm:not-sr-only">{label}</span>
    </span>
  );
}

function EmptyState({
  status,
  error,
  programId,
}: {
  status: ReturnType<typeof useGuardAccount>['status'];
  error: string | null;
  programId: string | null;
}) {
  const copy: Record<string, { title: string; body: string }> = {
    loading: {
      title: 'Connecting',
      body: 'Reading guard accounts from the configured RPC.',
    },
    config: {
      title: 'Not configured',
      body: 'Set NEXT_PUBLIC_GUARD_PROGRAM_ID (and optionally NEXT_PUBLIC_SOLANA_RPC) in .env.local, then reload. See .env.example.',
    },
    empty: {
      title: 'No guard found',
      body: 'The program is reachable but there is no PositionGuard at this address yet. Connect the owner wallet and initialize one — the guard PDA is derived from b"guard" || owner.',
    },
    error: {
      title: 'Cannot read guard state',
      body: error ?? 'The RPC request failed.',
    },
  };

  const { title, body } = copy[status] ?? copy.error!;

  return (
    <div className="mx-auto max-w-xl rounded-xl border border-border bg-surface/40 p-8 text-center">
      <h1 className="font-serif text-2xl text-foreground">{title}</h1>
      <p className="mt-3 text-sm leading-relaxed text-muted-foreground">{body}</p>
      {programId ? (
        <p className="mt-4 font-mono text-[11px] break-all text-muted-foreground/70">
          program {programId}
        </p>
      ) : null}
      <Link
        href="/"
        className="mt-6 inline-flex items-center gap-2 rounded-md border border-border px-4 py-2 text-sm text-foreground transition-colors hover:border-primary hover:text-primary"
      >
        <ArrowLeft className="h-4 w-4" />
        Back to overview
      </Link>
    </div>
  );
}
