/**
 * Health math, mirroring `program/src/state.rs`.
 *
 * The program never divides and never uses floats — it cross-multiplies in
 * fixed point. This module keeps every decision (liquidatable, breaching the
 * trigger buffer) in `bigint` on the same terms, and only converts to a float
 * at the very end, for display.
 */

import { BPS_DENOM, SCALE, type GuardState } from './guard-layout';

export function computePnl(size: bigint, entry: bigint, current: bigint): bigint {
  return (size * (current - entry)) / SCALE;
}

export function computeMarginRequired(absSize: bigint, marginBps: bigint): bigint {
  return (absSize * marginBps) / BPS_DENOM;
}

export function equity(collateral: bigint, pnl: bigint): bigint {
  return collateral + pnl;
}

function abs(v: bigint): bigint {
  return v < 0n ? -v : v;
}

export interface Health {
  pnl: bigint;
  equity: bigint;
  marginRequired: bigint;
  /** Margin plus the trigger buffer — the level the guard defends. */
  triggerTarget: bigint;
  notional: bigint;
  liquidatable: boolean;
  /** Equity is below the buffer but not yet below maintenance margin. */
  breachingBuffer: boolean;
  /** equity / marginRequired, for display only. */
  factor: number;
  /** The factor at which the guard fires — 1 + trigger_buffer_bps/10000. */
  triggerFactor: number;
}

export function computeHealth(state: GuardState): Health {
  const { collateral, size, entry, currentPrice, policy } = state;
  const absSize = abs(size);

  const pnl = computePnl(size, entry, currentPrice);
  const eq = equity(collateral, pnl);
  const marginRequired = computeMarginRequired(absSize, policy.maintenanceBps);
  const triggerTarget = (marginRequired * (BPS_DENOM + policy.triggerBufferBps)) / BPS_DENOM;
  const notional = (absSize * currentPrice) / SCALE;

  const liquidatable = eq < marginRequired;
  const breachingBuffer = !liquidatable && eq < triggerTarget;

  const factor = marginRequired === 0n ? Infinity : Number(eq) / Number(marginRequired);
  const triggerFactor = 1 + Number(policy.triggerBufferBps) / Number(BPS_DENOM);

  return {
    pnl,
    equity: eq,
    marginRequired,
    triggerTarget,
    notional,
    liquidatable,
    breachingBuffer,
    factor,
    triggerFactor,
  };
}

/** Fixed-point value (6dp) to a JS number. Display only. */
export function toNumber(v: bigint): number {
  return Number(v) / Number(SCALE);
}

export function formatUsd(v: bigint, decimals = 2): string {
  const n = toNumber(v);
  const sign = n < 0 ? '-' : '';
  return `${sign}$${Math.abs(n).toLocaleString('en-US', {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  })}`;
}

export function formatQty(v: bigint, decimals = 2): string {
  return toNumber(v).toLocaleString('en-US', {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  });
}

export function formatBps(bps: bigint): string {
  return `${Number(bps) / 100}%`;
}
