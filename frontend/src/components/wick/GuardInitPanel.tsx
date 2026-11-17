'use client';

import { Children, cloneElement, isValidElement, useId, useState, type ReactNode } from 'react';
import { AlertTriangle, ExternalLink } from 'lucide-react';
import { PublicKey } from '@solana/web3.js';
import { useWallet, useWalletAvailable } from '@/hooks/useWallet';
import type { InitGuardInput, OpKind, TxState } from '@/hooks/useGuardActions';
import type { RouteConfigState } from '@/hooks/useGuardAccount';
import { VENUE_DRIFT, VENUE_JUPITER, VENUE_NONE } from '@/lib/guard-layout';
import { parseUint, parseUsd } from '@/lib/amounts';
import { guardPda } from '@/lib/guard-program';
import { TxResult } from './WriteForms';
import { cn } from '@/lib/utils';

const VENUES = [
  { value: 'none', label: 'Watch only — no venue adapter', byte: VENUE_NONE },
  { value: 'jupiter', label: 'Jupiter — co-signed', byte: VENUE_JUPITER },
  { value: 'drift', label: 'Drift — autonomous', byte: VENUE_DRIFT },
] as const;

type VenueValue = (typeof VENUES)[number]['value'];

/**
 * First-run onboarding: create RouteConfig if the program has none, then create
 * the guard PDA for the connected wallet.
 *
 * The policy defaults are the same ones `cranker/src/init.mjs` uses, shown in a
 * collapsed section rather than hidden entirely — they are spend caps, and an
 * owner should be able to see and change them before the guard starts acting.
 */
export function GuardInitPanel({
  programId,
  routeConfig,
  canInit,
  tx,
  pending,
  onInitRouteConfig,
  onInitGuard,
}: {
  programId: string | null;
  routeConfig: RouteConfigState;
  canInit: boolean;
  tx: TxState;
  pending: (op: OpKind) => boolean;
  onInitRouteConfig: () => void;
  onInitGuard: (input: InitGuardInput) => void;
}) {
  const { publicKey } = useWallet();
  const available = useWalletAvailable();
  const [venue, setVenue] = useState<VenueValue>('none');
  const [coAuthority, setCoAuthority] = useState('');
  const [maintenance, setMaintenance] = useState('500');
  const [triggerBuffer, setTriggerBuffer] = useState('200');
  const [fee, setFee] = useState('10');
  const [capTopUp, setCapTopUp] = useState('1000');
  const [capPartialClose, setCapPartialClose] = useState('5000');
  const [capDaily, setCapDaily] = useState('20000');
  const [takeProfit, setTakeProfit] = useState('');
  const [marketIndex, setMarketIndex] = useState('0');
  const [subaccountId, setSubaccountId] = useState('0');
  const [error, setError] = useState<string | null>(null);

  const configReady = routeConfig.exists && !routeConfig.paused;
  const routeConfigMissing = !routeConfig.exists;

  const expectedGuard =
    programId && publicKey
      ? guardPda(new PublicKey(programId), publicKey)[0].toBase58()
      : null;
  const effectiveCoAuthority = coAuthority.trim() || publicKey?.toBase58() || '';

  const submit = () => {
    if (!publicKey) return setError('Connect a wallet first.');
    if (!configReady) {
      return setError('Initialize RouteConfig before creating a guard — writes reject without it.');
    }

    let coKey: PublicKey;
    try {
      coKey = new PublicKey(effectiveCoAuthority);
    } catch {
      return setError('Co-authority must be a valid base58 public key.');
    }

    const m = parseUint(maintenance);
    if (!m.ok) return setError(`Maintenance margin: ${m.error}`);
    const t = parseUint(triggerBuffer);
    if (!t.ok) return setError(`Trigger buffer: ${t.error}`);
    const f = parseUint(fee);
    if (!f.ok) return setError(`Fee assumption: ${f.error}`);
    const top = parseUsd(capTopUp);
    if (!top.ok) return setError(`Top-up cap: ${top.error}`);
    const partial = parseUsd(capPartialClose);
    if (!partial.ok) return setError(`Partial-close cap: ${partial.error}`);
    const daily = parseUsd(capDaily);
    if (!daily.ok) return setError(`Daily cap: ${daily.error}`);

    let tp: bigint | null = null;
    if (takeProfit.trim() !== '') {
      const parsed = parseUsd(takeProfit);
      if (!parsed.ok) return setError(`Take profit: ${parsed.error}`);
      tp = parsed.value;
    }

    const market = parseUint(marketIndex);
    if (!market.ok) return setError(`Market index: ${market.error}`);
    if (market.value > 0xffffn) return setError('Market index must fit in u16.');
    const sub = parseUint(subaccountId);
    if (!sub.ok) return setError(`Sub-account: ${sub.error}`);
    if (sub.value > 0xffffn) return setError('Sub-account must fit in u16.');

    setError(null);
    onInitGuard({
      venue: VENUES.find((v) => v.value === venue)?.byte ?? VENUE_NONE,
      coAuthority: coKey.toBase58(),
      authorityReq: venue === 'drift' ? 0 : 1,
      maintenanceBps: m.value,
      triggerBufferBps: t.value,
      feeBps: f.value,
      topUpUsdPerAction: top.value,
      partialCloseUsdPerAction: partial.value,
      dailyTotalUsd: daily.value,
      takeProfit: tp,
      driftMarketIndex: Number(market.value),
      driftSubaccountId: Number(sub.value),
    });
  };

  return (
    <div className="mx-auto max-w-2xl space-y-4">
      <div className="rounded-xl border border-border bg-surface/40 p-6">
        <h1 className="font-serif text-2xl text-foreground">Create your guard</h1>
        <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
          This wallet has no guard yet. The guard PDA is derived from your key, so this address is
          the only one it will ever have. Creating it records the policy the guard will act under.
        </p>

        {!publicKey ? (
          <div className="mt-5 rounded-md border border-warning/40 bg-warning/5 px-3 py-3">
            <p className="text-[12.5px] leading-relaxed text-warning">
              Connect a Solana wallet to create a guard.
            </p>
            {!available ? <WalletLinks /> : null}
          </div>
        ) : (
          <>
            {routeConfigMissing ? (
              <div className="mt-5 rounded-md border border-warning/40 bg-warning/5 px-3 py-3">
                <p className="flex items-center gap-1.5 font-mono text-[11px] tracking-[0.08em] text-warning">
                  <AlertTriangle className="h-3.5 w-3.5" aria-hidden="true" />
                  PROGRAM SETUP REQUIRED
                </p>
                <p className="mt-2 text-[12.5px] leading-relaxed text-muted-foreground">
                  RouteConfig is missing or was written by an older program version, so every
                  state-mutating instruction rejects until it exists.
                </p>
                <p className="mt-2 text-[12px] leading-relaxed text-muted-foreground">
                  Initializing it makes <span className="text-foreground">this wallet</span> the
                  kill-switch authority. Only do this on a fresh deployment — an initialized config
                  is refused, so this cannot take over a live one.
                </p>
                <button
                  type="button"
                  onClick={onInitRouteConfig}
                  disabled={!canInit || pending('initRouteConfig')}
                  className={cn(
                    'mt-3 w-full rounded-md border px-3 py-2 font-mono text-[11.5px] tracking-[0.06em] transition-colors',
                    !canInit || pending('initRouteConfig')
                      ? 'cursor-not-allowed border-border text-muted-foreground'
                      : 'border-warning/60 text-warning hover:bg-warning/10',
                  )}
                >
                  {pending('initRouteConfig') ? 'sending…' : 'initialize route config'}
                </button>
                <TxResult tx={tx} op="initRouteConfig" />
              </div>
            ) : null}

            <form
              onSubmit={(e) => {
                e.preventDefault();
                submit();
              }}
              className="mt-5 space-y-4"
            >
              <Field label="Venue">
                <select
                  value={venue}
                  onChange={(e) => setVenue(e.target.value as VenueValue)}
                  disabled={pending('initGuard')}
                  className={inputClass}
                >
                  {VENUES.map((v) => (
                    <option key={v.value} value={v.value}>
                      {v.label}
                    </option>
                  ))}
                </select>
                <p className="mt-1 text-[11.5px] leading-relaxed text-muted-foreground">
                  {venue === 'drift'
                    ? 'Autonomous: the guard PDA is the delegate and signs its own reduce. A top-up needs a funded reserve.'
                    : 'Co-signed: the guard builds the instruction and the owner’s signature lands it.'}
                </p>
              </Field>

              <Field label="Co-authority">
                <input
                  value={effectiveCoAuthority}
                  onChange={(e) => {
                    setCoAuthority(e.target.value);
                    if (error) setError(null);
                  }}
                  disabled={pending('initGuard')}
                  spellCheck={false}
                  className={inputClass}
                />
                <p className="mt-1 text-[11.5px] leading-relaxed text-muted-foreground">
                  2-of-2 withdrawals need a second key. It defaults to this wallet so a devnet guard
                  works with one keypair; a real deployment should use a separately held key.
                </p>
              </Field>

              {venue === 'drift' ? (
                <div className="grid gap-3 sm:grid-cols-2">
                  <Field label="Drift market index">
                    <input
                      value={marketIndex}
                      onChange={(e) => setMarketIndex(e.target.value)}
                      inputMode="numeric"
                      disabled={pending('initGuard')}
                      className={inputClass}
                    />
                  </Field>
                  <Field label="Drift sub-account">
                    <input
                      value={subaccountId}
                      onChange={(e) => setSubaccountId(e.target.value)}
                      inputMode="numeric"
                      disabled={pending('initGuard')}
                      className={inputClass}
                    />
                  </Field>
                </div>
              ) : null}

              <details className="rounded-md border border-border px-3 py-2.5">
                <summary className="cursor-pointer font-mono text-[11px] tracking-[0.08em] text-muted-foreground">
                  ADVANCED POLICY
                </summary>
                <p className="mt-2 text-[11.5px] leading-relaxed text-muted-foreground">
                  Defaults match the cranker bring-up. Caps are USD per action / per day; bps are
                  basis points.
                </p>
                <div className="mt-3 grid gap-3 sm:grid-cols-2">
                  <Field label="Maintenance margin (bps)">
                    <input
                      value={maintenance}
                      onChange={(e) => setMaintenance(e.target.value)}
                      inputMode="numeric"
                      disabled={pending('initGuard')}
                      className={inputClass}
                    />
                  </Field>
                  <Field label="Trigger buffer (bps)">
                    <input
                      value={triggerBuffer}
                      onChange={(e) => setTriggerBuffer(e.target.value)}
                      inputMode="numeric"
                      disabled={pending('initGuard')}
                      className={inputClass}
                    />
                  </Field>
                  <Field label="Fee assumption (bps)">
                    <input
                      value={fee}
                      onChange={(e) => setFee(e.target.value)}
                      inputMode="numeric"
                      disabled={pending('initGuard')}
                      className={inputClass}
                    />
                  </Field>
                  <Field label="Take profit (USD, optional)">
                    <input
                      value={takeProfit}
                      onChange={(e) => setTakeProfit(e.target.value)}
                      inputMode="decimal"
                      placeholder="none"
                      disabled={pending('initGuard')}
                      className={inputClass}
                    />
                  </Field>
                  <Field label="Top-up cap (USD / action)">
                    <input
                      value={capTopUp}
                      onChange={(e) => setCapTopUp(e.target.value)}
                      inputMode="decimal"
                      disabled={pending('initGuard')}
                      className={inputClass}
                    />
                  </Field>
                  <Field label="Partial-close cap (USD / action)">
                    <input
                      value={capPartialClose}
                      onChange={(e) => setCapPartialClose(e.target.value)}
                      inputMode="decimal"
                      disabled={pending('initGuard')}
                      className={inputClass}
                    />
                  </Field>
                  <Field label="Daily cap (USD)">
                    <input
                      value={capDaily}
                      onChange={(e) => setCapDaily(e.target.value)}
                      inputMode="decimal"
                      disabled={pending('initGuard')}
                      className={inputClass}
                    />
                  </Field>
                </div>
              </details>

              {expectedGuard ? (
                <p className="font-mono text-[10.5px] break-all text-muted-foreground/70">
                  creates guard {expectedGuard}
                </p>
              ) : null}

              {error ? (
                <p role="alert" className="font-mono text-[11px] text-risk">
                  {error}
                </p>
              ) : null}

              <button
                type="submit"
                disabled={!canInit || !configReady || pending('initGuard')}
                className={cn(
                  'w-full rounded-md border px-3 py-2.5 font-mono text-[12px] tracking-[0.06em] transition-colors',
                  !canInit || !configReady || pending('initGuard')
                    ? 'cursor-not-allowed border-border text-muted-foreground'
                    : 'border-primary/60 text-primary hover:bg-primary/10',
                )}
              >
                {pending('initGuard') ? 'creating…' : 'create guard'}
              </button>
              {!configReady ? (
                <p className="text-[11.5px] leading-relaxed text-muted-foreground">
                  RouteConfig has to exist and be unpaused before the guard can be written to.
                </p>
              ) : null}
              <TxResult tx={tx} op="initGuard" />
            </form>
          </>
        )}
      </div>
    </div>
  );
}

function WalletLinks() {
  return (
    <p className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[12px] text-muted-foreground">
      <span>No wallet detected. Install</span>
      <a
        href="https://phantom.app/"
        target="_blank"
        rel="noreferrer"
        className="inline-flex items-center gap-1 text-foreground hover:text-primary"
      >
        Phantom <ExternalLink className="h-3 w-3" aria-hidden="true" />
      </a>
      <span>or</span>
      <a
        href="https://solflare.com/"
        target="_blank"
        rel="noreferrer"
        className="inline-flex items-center gap-1 text-foreground hover:text-primary"
      >
        Solflare <ExternalLink className="h-3 w-3" aria-hidden="true" />
      </a>
    </p>
  );
}

const inputClass =
  'w-full rounded-md border border-border bg-background px-2.5 py-2 font-mono text-[12px] tabular-nums text-foreground placeholder:text-muted-foreground/50 focus:border-primary focus:outline-none disabled:cursor-not-allowed disabled:opacity-50';

function Field({ label, children }: { label: string; children: ReactNode }) {
  const id = useId();
  const [control, ...rest] = Children.toArray(children);
  // Give the first child the generated id so the visible label is the real
  // accessible name; helper text stays outside the control.
  const labelled = isValidElement<{ id?: string }>(control)
    ? cloneElement(control, { id })
    : control;

  return (
    <div>
      <label htmlFor={id} className="block font-mono text-[10px] text-muted-foreground/70">
        {label}
      </label>
      <div className="mt-1">
        {labelled}
        {rest}
      </div>
    </div>
  );
}
