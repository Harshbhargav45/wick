'use client';

import { useId, useState, type ReactNode } from 'react';
import type { OpKind, TxState } from '@/hooks/useGuardActions';
import { cn } from '@/lib/utils';
import type { ParseResult } from '@/lib/amounts';

/**
 * The result of one write, scoped to the operation that produced it.
 *
 * Every state here is reported as what it is. `awaitingCoSign` in particular is
 * neither success nor failure: the transaction exists, carries the owner's
 * signature, and has not been sent — calling that "sent" would tell an owner
 * value moved when nothing has.
 */
export function TxResult({ tx, op }: { tx: TxState; op: OpKind }) {
  if (tx.kind === 'idle' || tx.op !== op) return null;

  if (tx.kind === 'error') {
    return (
      <p role="alert" className="mt-2 font-mono text-[11px] leading-relaxed break-all text-risk">
        {tx.message}
      </p>
    );
  }

  if (tx.kind === 'sent') {
    return (
      <p className="mt-2 font-mono text-[11px] break-all text-healthy">
        confirmed · {tx.signature.slice(0, 16)}…
      </p>
    );
  }

  if (tx.kind === 'awaitingCoSign') {
    return <CoSignHandoff base64={tx.base64} />;
  }

  return null;
}

/**
 * The owner's half of a 2-of-2, ready to hand to the co-authority.
 *
 * Deliberately a copyable blob rather than a send button. The console holds one
 * of the two required keys, so there is nothing it can do here but produce the
 * bytes honestly and say so — broadcasting would be rejected by the program,
 * and a spinner over a transaction that cannot land is worse than a hand-off.
 */
function CoSignHandoff({ base64 }: { base64: string }) {
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(base64);
      setCopied(true);
    } catch {
      // A clipboard the browser refuses is not worth an error banner — the
      // textarea below is still selectable by hand.
      setCopied(false);
    }
  };

  return (
    <div className="mt-3 rounded-md border border-warning/40 bg-warning/5 p-3">
      <p className="font-mono text-[11px] tracking-[0.06em] text-warning">
        signed by owner · awaiting co-authority
      </p>
      <p className="mt-2 text-[11.5px] leading-relaxed text-muted-foreground">
        This is a 2-of-2 instruction. It carries your signature and has{' '}
        <span className="text-foreground">not</span> been sent — the co-authority has to sign the
        same bytes before it can land.
      </p>
      <textarea
        readOnly
        value={base64}
        rows={3}
        aria-label="Partially signed transaction, base64"
        className="mt-2 w-full resize-none rounded border border-border bg-background px-2 py-1.5 font-mono text-[10px] break-all text-muted-foreground"
      />
      <button
        type="button"
        onClick={copy}
        className="mt-2 rounded border border-border px-2.5 py-1 font-mono text-[11px] text-muted-foreground transition-colors hover:border-warning hover:text-warning"
      >
        {copied ? 'copied' : 'copy base64'}
      </button>
    </div>
  );
}

/**
 * A single-amount write form: label, unit-suffixed input, submit.
 *
 * Validation runs on submit rather than on every keystroke, so a half-typed
 * "0." is not an error yet. `parse` returns the scaled `bigint` the program
 * takes, so no component downstream of here handles a decimal string.
 */
export function AmountForm({
  label,
  unit,
  placeholder,
  parse,
  onSubmit,
  submitLabel,
  disabled,
  disabledReason,
  pending,
  tone = 'default',
  help,
  children,
}: {
  label: string;
  unit: string;
  placeholder?: string;
  parse: (input: string) => ParseResult;
  onSubmit: (value: bigint) => void;
  submitLabel: string;
  disabled?: boolean;
  disabledReason?: string | null;
  pending?: boolean;
  tone?: 'default' | 'warning';
  help?: ReactNode;
  children?: ReactNode;
}) {
  const id = useId();
  const [text, setText] = useState('');
  const [error, setError] = useState<string | null>(null);

  const submit = () => {
    const parsed = parse(text);
    if (!parsed.ok) {
      setError(parsed.error);
      return;
    }
    setError(null);
    onSubmit(parsed.value);
    setText('');
  };

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
      className="space-y-2"
    >
      <label htmlFor={id} className="block font-mono text-[11px] text-muted-foreground">
        {label}
      </label>
      <div className="flex gap-2">
        <div className="relative flex-1">
          <input
            id={id}
            value={text}
            onChange={(e) => {
              setText(e.target.value);
              if (error) setError(null);
            }}
            inputMode="decimal"
            autoComplete="off"
            placeholder={placeholder}
            disabled={disabled || pending}
            aria-invalid={error !== null}
            aria-describedby={error ? `${id}-error` : undefined}
            className={cn(
              'w-full rounded-md border bg-background px-2.5 py-2 pr-12 font-mono text-[12px] tabular-nums text-foreground',
              'placeholder:text-muted-foreground/50 focus:outline-none',
              error ? 'border-risk focus:border-risk' : 'border-border focus:border-primary',
              (disabled || pending) && 'cursor-not-allowed opacity-50',
            )}
          />
          <span className="pointer-events-none absolute top-1/2 right-2.5 -translate-y-1/2 font-mono text-[10px] text-muted-foreground/70">
            {unit}
          </span>
        </div>
        <button
          type="submit"
          disabled={disabled || pending}
          className={cn(
            'shrink-0 rounded-md border px-3 py-2 font-mono text-[11.5px] tracking-[0.06em] transition-colors',
            disabled || pending
              ? 'cursor-not-allowed border-border text-muted-foreground'
              : tone === 'warning'
                ? 'border-warning/60 text-warning hover:bg-warning/10'
                : 'border-border text-foreground hover:border-primary hover:text-primary',
          )}
        >
          {pending ? 'sending…' : submitLabel}
        </button>
      </div>
      {error ? (
        <p id={`${id}-error`} role="alert" className="font-mono text-[11px] text-risk">
          {error}
        </p>
      ) : null}
      {disabled && disabledReason ? (
        <p className="text-[11.5px] leading-relaxed text-muted-foreground">{disabledReason}</p>
      ) : null}
      {help ? <p className="text-[11.5px] leading-relaxed text-muted-foreground">{help}</p> : null}
      {children}
    </form>
  );
}

/**
 * A button for an action that cannot be undone, which requires a second click
 * to arm.
 *
 * `CloseGuard` zeroes the account and refunds the rent — an owner who meant to
 * press Refresh does not get to take it back, and the guard PDA is a pure
 * function of the owner's key, so the same address is all they will ever have.
 */
export function DestructiveButton({
  label,
  confirmLabel,
  warning,
  onConfirm,
  disabled,
  pending,
}: {
  label: string;
  confirmLabel: string;
  warning: ReactNode;
  onConfirm: () => void;
  disabled?: boolean;
  pending?: boolean;
}) {
  const [armed, setArmed] = useState(false);

  if (!armed) {
    return (
      <button
        type="button"
        onClick={() => setArmed(true)}
        disabled={disabled || pending}
        className={cn(
          'w-full rounded-md border px-3 py-2 font-mono text-[11.5px] tracking-[0.06em] transition-colors',
          disabled || pending
            ? 'cursor-not-allowed border-border text-muted-foreground'
            : 'border-border text-muted-foreground hover:border-risk hover:text-risk',
        )}
      >
        {label}
      </button>
    );
  }

  return (
    <div className="rounded-md border border-risk/50 bg-risk/5 p-3">
      <p className="text-[11.5px] leading-relaxed text-foreground">{warning}</p>
      <div className="mt-3 flex gap-2">
        <button
          type="button"
          onClick={onConfirm}
          disabled={pending}
          className={cn(
            'flex-1 rounded-md border border-risk px-3 py-2 font-mono text-[11.5px] tracking-[0.06em] text-risk transition-colors',
            pending ? 'cursor-not-allowed opacity-60' : 'hover:bg-risk/10',
          )}
        >
          {pending ? 'sending…' : confirmLabel}
        </button>
        <button
          type="button"
          onClick={() => setArmed(false)}
          className="rounded-md border border-border px-3 py-2 font-mono text-[11.5px] text-muted-foreground transition-colors hover:text-foreground"
        >
          cancel
        </button>
      </div>
    </div>
  );
}
