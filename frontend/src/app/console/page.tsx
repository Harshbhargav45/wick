'use client';

import Link from 'next/link';
import { PublicKey } from '@solana/web3.js';
import { ArrowLeft, RefreshCw } from 'lucide-react';
import { useGuardAccount, writeBlockReason } from '@/hooks/useGuardAccount';
import { useGuardEvents } from '@/hooks/useGuardEvents';
import { useGuardActions, type OpKind } from '@/hooks/useGuardActions';
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
import { ReconcilePanel } from '@/components/wick/ReconcilePanel';
import { ReservePanel } from '@/components/wick/ReservePanel';
import { PositionActions } from '@/components/wick/PositionActions';
import { latencyStats } from '@/lib/wick-data';
import { cn } from '@/lib/utils';

export default function ConsolePage() {
  const { snapshot, status, error, routeConfig, writesBlocked, refresh, rpc, programId } =
    useGuardAccount();
  const events = useGuardEvents(snapshot);
  const { publicKey } = useWallet();
  const actions = useGuardActions({
    programId,
    guardAddress: snapshot?.address ?? null,
    owner: publicKey,
    // Read off the guard rather than assumed, because the 2-of-2 paths must
    // name the exact key the program will compare against.
    coAuthority: snapshot ? new PublicKey(snapshot.state.coAuthority).toBase58() : null,
    marginWalletBump: snapshot?.state.marginWalletBump ?? null,
    onDone: refresh,
  });
  const { tx, canSend, isPending, canCoSign } = actions;

  // One reason, one place. Every disabled control quotes the same sentence for
  // the same condition instead of inventing its own.
  const blockReason = writeBlockReason(routeConfig, snapshot, Boolean(publicKey));
  const canWrite = canSend && blockReason === null && !writesBlocked;
  /**
   * Pending is per-operation: `isPending` covers the click-to-wallet window that
   * `tx.kind` misses, but it is global to the hook, so it is only attributed to
   * the operation the current `tx` belongs to. Otherwise one click would show
   * every button in the console as sending.
   */
  const pending = (op: OpKind): boolean =>
    tx.kind !== 'idle' && tx.op === op && (isPending || tx.kind === 'sending');

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
                  guard {snapshot.address.slice(0, 6)}…{snapshot.address.slice(-6)} · last tick{' '}
                  {describeTick(snapshot.state.lastCheckTs, snapshot.chainTs)}
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
                  canConfirm={canWrite}
                  blockReason={blockReason}
                  onConfirm={actions.confirm}
                  tx={tx}
                  pending={pending('confirm')}
                  diverged={snapshot.health.diverged}
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
                <ReconcilePanel state={snapshot.state} chainTs={snapshot.chainTs} />
                <PolicyPanel state={snapshot.state} budget={snapshot.budget} />
                <ReservePanel
                  reserve={snapshot.reserve}
                  topUpCap={snapshot.state.policy.caps.topUpUsdPerAction}
                  canWrite={canWrite}
                  blockReason={blockReason}
                  canCoSign={canCoSign}
                  onInit={actions.initReserve}
                  onFund={actions.fundReserve}
                  onWithdraw={actions.withdrawReserve}
                  tx={tx}
                  pending={pending}
                />
                <PositionActions
                  state={snapshot.state}
                  diverged={snapshot.health.diverged}
                  canWrite={canWrite}
                  blockReason={blockReason}
                  canCoSign={canCoSign}
                  onDeposit={actions.deposit}
                  onWithdraw={actions.withdraw}
                  onUpdatePosition={actions.updatePosition}
                  onCloseGuard={actions.closeGuard}
                  tx={tx}
                  pending={pending}
                />
              </div>
            </div>
          </>
        )}
      </main>

      <footer className="mx-auto mt-4 flex max-w-6xl flex-wrap items-center justify-between gap-3 border-t border-border px-4 py-6 font-mono text-[11px] text-muted-foreground sm:px-6">
        <span>
          guard state is polled every 5s from{' '}
          {/* Redacted at the source in `rpc.ts` — a keyed endpoint is a
              credential and does not belong in the DOM. */}
          <span className="text-muted-foreground/80">{rpc}</span>
        </span>
        <PoweredByMagicBlock />
      </footer>
    </div>
  );
}

/**
 * Age of the last accepted tick, against the chain's clock.
 *
 * `last_check_ts` is unix seconds, not a slot — printing the raw number was both
 * mislabelled and unreadable. Compared against the `Clock` sysvar rather than
 * `Date.now()`, so a browser with a skewed system time does not report a fresh
 * guard as minutes stale.
 */
function describeTick(lastCheckTs: bigint, chainTs: bigint): string {
  if (lastCheckTs === 0n) return 'never ticked';
  const age = chainTs - lastCheckTs;
  if (age <= 0n) return 'just now';
  if (age < 60n) return `${age}s ago`;
  if (age < 3_600n) return `${age / 60n}m ago`;
  return `${age / 3_600n}h ago`;
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
