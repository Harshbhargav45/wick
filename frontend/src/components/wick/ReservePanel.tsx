'use client';

import type { ReserveState } from '@/hooks/useGuardAccount';
import type { OpKind, TxState } from '@/hooks/useGuardActions';
import { formatSol, parseSol } from '@/lib/amounts';
import { AmountForm, TxResult } from './WriteForms';
import { cn } from '@/lib/utils';

/**
 * The §8.5 lamport reserve behind autonomous top-ups.
 *
 * This panel exists because "the policy allows a $500 top-up" and "there are
 * lamports to top up with" are different facts, and only the second one stops a
 * breach. A guard with a TopUp cap and no reserve escalates to manual review at
 * the moment it was supposed to act, so the absence of a reserve is shown as a
 * warning rather than an empty state.
 */
export function ReservePanel({
  reserve,
  topUpCap,
  canWrite,
  blockReason,
  canCoSign,
  onInit,
  onFund,
  onWithdraw,
  tx,
  pending,
}: {
  reserve: ReserveState;
  /** The policy's per-action top-up cap, to say whether the reserve can cover one. */
  topUpCap: bigint;
  canWrite: boolean;
  blockReason: string | null;
  canCoSign: boolean;
  onInit: () => void;
  onFund: (lamports: bigint) => void;
  onWithdraw: (lamports: bigint) => void;
  tx: TxState;
  pending: (op: OpKind) => boolean;
}) {
  const rentLocked = reserve.lamports > reserve.balance ? reserve.lamports - reserve.balance : 0n;

  return (
    <div
      className={cn(
        'rounded-xl border bg-surface/40',
        reserve.exists ? 'border-border' : 'border-warning/40',
      )}
    >
      <div className="flex items-center justify-between border-b border-border px-4 py-3">
        <span className="font-mono text-[11px] tracking-[0.24em] text-muted-foreground">
          MARGIN RESERVE
        </span>
        <span
          className={cn(
            'font-mono text-[11px]',
            reserve.exists ? 'text-healthy' : 'text-warning',
          )}
        >
          {reserve.exists ? 'linked' : 'none'}
        </span>
      </div>

      <div className="px-4 py-4">
        {reserve.exists ? (
          <>
            <dl className="space-y-2.5 text-[12px]">
              <div className="flex items-baseline justify-between gap-3">
                <dt className="text-muted-foreground">Withdrawable</dt>
                <dd className="font-mono tabular-nums text-foreground">
                  {formatSol(reserve.balance)} SOL
                </dd>
              </div>
              <div className="flex items-baseline justify-between gap-3">
                <dt className="text-muted-foreground">Rent (not withdrawable)</dt>
                <dd className="font-mono tabular-nums text-muted-foreground">
                  {formatSol(rentLocked, 6)} SOL
                </dd>
              </div>
            </dl>
            <p className="mt-2 font-mono text-[10px] break-all text-muted-foreground/60">
              {reserve.address}
            </p>

            {reserve.balance === 0n && topUpCap > 0n ? (
              <p className="mt-3 rounded-md border border-warning/40 bg-warning/5 px-2.5 py-2 text-[11.5px] leading-relaxed text-warning">
                The reserve is empty. The policy permits a top-up, but there are no lamports behind
                it — the guard will escalate instead of acting.
              </p>
            ) : null}

            <div className="mt-4 space-y-4 border-t border-border pt-4">
              <AmountForm
                label="Fund reserve"
                unit="SOL"
                placeholder="0.5"
                parse={parseSol}
                onSubmit={onFund}
                submitLabel="fund"
                disabled={!canWrite}
                disabledReason={blockReason}
                pending={pending('fundReserve')}
                help="Moves real lamports from your wallet into the reserve."
              />
              <TxResult tx={tx} op="fundReserve" />

              <AmountForm
                label="Withdraw reserve"
                unit="SOL"
                placeholder="0.5"
                parse={parseSol}
                onSubmit={onWithdraw}
                submitLabel="withdraw"
                tone="warning"
                disabled={!canWrite || !canCoSign}
                disabledReason={
                  !canWrite
                    ? blockReason
                    : 'This wallet cannot sign without sending, so the owner half of the 2-of-2 cannot be produced here.'
                }
                pending={pending('withdrawReserve')}
                help="2-of-2 — produces a partially signed transaction for the co-authority rather than sending."
              />
              <TxResult tx={tx} op="withdrawReserve" />
            </div>
          </>
        ) : (
          <>
            <p className="text-[12px] leading-relaxed text-muted-foreground">
              No reserve is linked to this guard. An autonomous top-up needs lamports to move, so
              until one exists the guard records a TopUp action and escalates rather than executing
              it.
            </p>
            <button
              type="button"
              onClick={onInit}
              disabled={!canWrite || pending('initReserve')}
              className={cn(
                'mt-4 w-full rounded-md border px-3 py-2 font-mono text-[11.5px] tracking-[0.06em] transition-colors',
                !canWrite || pending('initReserve')
                  ? 'cursor-not-allowed border-border text-muted-foreground'
                  : 'border-warning/60 text-warning hover:bg-warning/10',
              )}
            >
              {pending('initReserve') ? 'sending…' : 'create reserve'}
            </button>
            {!canWrite && blockReason ? (
              <p className="mt-2 text-[11.5px] leading-relaxed text-muted-foreground">
                {blockReason}
              </p>
            ) : null}
            <TxResult tx={tx} op="initReserve" />
          </>
        )}
      </div>
    </div>
  );
}
