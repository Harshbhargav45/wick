'use client';

import { useId, useState } from 'react';
import type { GuardState } from '@/lib/guard-layout';
import type { OpKind, TxState } from '@/hooks/useGuardActions';
import { formatUsd } from '@/lib/guard-health';
import { parseSignedDecimal, parseUsd, USD_DECIMALS } from '@/lib/amounts';
import { AmountForm, DestructiveButton, TxResult } from './WriteForms';
import { cn } from '@/lib/utils';

/**
 * Owner-signed writes against the guard's own recorded state.
 *
 * `collateral` here is the guard's *model* of the position, not a token
 * balance — `DepositMargin` credits a number the health math runs on. That is
 * worth being explicit about on screen, because an owner who reads it as a
 * deposit will expect their wallet to be debited and it will not be.
 */
export function PositionActions({
  state,
  diverged,
  canWrite,
  blockReason,
  canCoSign,
  onDeposit,
  onWithdraw,
  onUpdatePosition,
  onCloseGuard,
  tx,
  pending,
}: {
  state: GuardState;
  diverged: boolean;
  canWrite: boolean;
  blockReason: string | null;
  canCoSign: boolean;
  onDeposit: (amount: bigint) => void;
  onWithdraw: (amount: bigint) => void;
  onUpdatePosition: (p: { collateral: bigint; size: bigint; entry: bigint }) => void;
  onCloseGuard: () => void;
  tx: TxState;
  pending: (op: OpKind) => boolean;
}) {
  return (
    <div className="rounded-xl border border-border bg-surface/40">
      <div className="border-b border-border px-4 py-3">
        <span className="font-mono text-[11px] tracking-[0.24em] text-muted-foreground">
          OWNER ACTIONS
        </span>
      </div>

      <div className="space-y-5 px-4 py-4">
        <div>
          <AmountForm
            label="Credit collateral"
            unit="USD"
            placeholder="500.00"
            parse={parseUsd}
            onSubmit={onDeposit}
            submitLabel="credit"
            disabled={!canWrite}
            disabledReason={blockReason}
            pending={pending('deposit')}
            help="Updates the collateral the guard runs its health math on. This is the guard's model of your position — it does not move tokens."
          />
          <TxResult tx={tx} op="deposit" />
        </div>

        <div className="border-t border-border pt-5">
          <AmountForm
            label="Debit collateral"
            unit="USD"
            placeholder="500.00"
            parse={parseUsd}
            onSubmit={onWithdraw}
            submitLabel="debit"
            tone="warning"
            disabled={!canWrite || !canCoSign}
            disabledReason={
              !canWrite
                ? blockReason
                : 'This wallet cannot sign without sending, so the owner half of the 2-of-2 cannot be produced here.'
            }
            pending={pending('withdraw')}
            help="2-of-2 — produces a partially signed transaction for the co-authority rather than sending."
          />
          <TxResult tx={tx} op="withdraw" />
        </div>

        <div className="border-t border-border pt-5">
          <PositionForm
            state={state}
            diverged={diverged}
            canWrite={canWrite}
            blockReason={blockReason}
            onSubmit={onUpdatePosition}
            pending={pending('updatePosition')}
          />
          <TxResult tx={tx} op="updatePosition" />
        </div>

        <div className="border-t border-border pt-5">
          <p className="mb-2 font-mono text-[11px] text-muted-foreground">Close guard</p>
          <DestructiveButton
            label="close guard and reclaim rent"
            confirmLabel="close permanently"
            warning={
              <>
                This zeroes the guard account and refunds its rent. The guard PDA is derived from
                your key, so this is the only address you get — a new guard has to be initialized
                from scratch, and nothing is watching the position in between.
              </>
            }
            onConfirm={onCloseGuard}
            disabled={!canWrite}
            pending={pending('closeGuard')}
          />
          {!canWrite && blockReason ? (
            <p className="mt-2 text-[11.5px] leading-relaxed text-muted-foreground">
              {blockReason}
            </p>
          ) : null}
          <TxResult tx={tx} op="closeGuard" />
        </div>
      </div>
    </div>
  );
}

/**
 * Re-enroll the watched position: collateral, signed size, entry.
 *
 * Three fields rather than three separate instructions because the program takes
 * them as one payload and overwrites all three — submitting a partial update
 * would zero the two fields left out. This is also the remedy for a diverged
 * guard, which is why it says so when one is.
 */
function PositionForm({
  state,
  diverged,
  canWrite,
  blockReason,
  onSubmit,
  pending,
}: {
  state: GuardState;
  diverged: boolean;
  canWrite: boolean;
  blockReason: string | null;
  onSubmit: (p: { collateral: bigint; size: bigint; entry: bigint }) => void;
  pending: boolean;
}) {
  const baseId = useId();
  const [collateral, setCollateral] = useState('');
  const [size, setSize] = useState('');
  const [entry, setEntry] = useState('');
  const [error, setError] = useState<string | null>(null);

  const submit = () => {
    const c = parseUsd(collateral);
    if (!c.ok) return setError(`Collateral: ${c.error}`);
    const s = parseSignedDecimal(size, USD_DECIMALS);
    if (!s.ok) return setError(`Size: ${s.error}`);
    const e = parseUsd(entry);
    if (!e.ok) return setError(`Entry: ${e.error}`);
    setError(null);
    onSubmit({ collateral: c.value, size: s.value, entry: e.value });
  };

  const fields: [string, string, string, (v: string) => void, string][] = [
    ['collateral', 'Collateral', collateral, setCollateral, 'USD'],
    ['size', 'Size (negative = short)', size, setSize, 'units'],
    ['entry', 'Entry price', entry, setEntry, 'USD'],
  ];

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
      className="space-y-2"
    >
      <p className="font-mono text-[11px] text-muted-foreground">Update position</p>

      {diverged ? (
        <p className="rounded-md border border-risk/40 bg-risk/5 px-2.5 py-2 text-[11.5px] leading-relaxed text-risk">
          The guard&apos;s position disagrees with the venue, so autonomous execution is blocked.
          Re-enrolling the real numbers here is what clears it.
        </p>
      ) : null}

      <div className="space-y-2">
        {fields.map(([key, label, value, set, unit]) => (
          <div key={key}>
            <label
              htmlFor={`${baseId}-${key}`}
              className="block font-mono text-[10px] text-muted-foreground/70"
            >
              {label}
            </label>
            <div className="relative mt-1">
              <input
                id={`${baseId}-${key}`}
                value={value}
                onChange={(ev) => {
                  set(ev.target.value);
                  if (error) setError(null);
                }}
                inputMode="decimal"
                autoComplete="off"
                disabled={!canWrite || pending}
                className={cn(
                  'w-full rounded-md border bg-background px-2.5 py-2 pr-14 font-mono text-[12px] tabular-nums text-foreground',
                  'placeholder:text-muted-foreground/50 focus:border-primary focus:outline-none',
                  error ? 'border-risk' : 'border-border',
                  (!canWrite || pending) && 'cursor-not-allowed opacity-50',
                )}
              />
              <span className="pointer-events-none absolute top-1/2 right-2.5 -translate-y-1/2 font-mono text-[10px] text-muted-foreground/70">
                {unit}
              </span>
            </div>
          </div>
        ))}
      </div>

      <button
        type="submit"
        disabled={!canWrite || pending}
        className={cn(
          'w-full rounded-md border px-3 py-2 font-mono text-[11.5px] tracking-[0.06em] transition-colors',
          !canWrite || pending
            ? 'cursor-not-allowed border-border text-muted-foreground'
            : 'border-border text-foreground hover:border-primary hover:text-primary',
        )}
      >
        {pending ? 'sending…' : 'update position'}
      </button>

      {error ? (
        <p role="alert" className="font-mono text-[11px] text-risk">
          {error}
        </p>
      ) : null}
      <p className="text-[11.5px] leading-relaxed text-muted-foreground">
        Overwrites all three fields at once. Currently {formatUsd(state.collateral)} collateral at{' '}
        {formatUsd(state.entry, 4)} entry.
      </p>
      {!canWrite && blockReason ? (
        <p className="text-[11.5px] leading-relaxed text-muted-foreground">{blockReason}</p>
      ) : null}
    </form>
  );
}
