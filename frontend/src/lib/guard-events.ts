import { BPS_DENOM, type Action } from './guard-layout';
import { formatUsd } from './guard-health';

/** Human-readable form of a pending `Action`, matching the program's variants. */
export function describeActionText(action: Action | null): string {
  if (!action) return 'none';
  switch (action.kind) {
    case 'TopUp':
      return `Top up ${formatUsd(action.amount)}`;
    case 'PartialClose':
      return `Partial close ${((Number(action.fractionBps) / Number(BPS_DENOM)) * 100).toFixed(2)}%`;
    case 'TakeProfit':
      return 'Take profit';
    case 'EscalateManualReview':
      return 'Escalate for manual review';
  }
}
