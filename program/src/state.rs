//! On-chain state layouts for the Wick guard program (Phase 1, no venues yet).
//!
//! All math §8.1 is defined here so unit tests can exercise it without needing
//! a full account/instruction context.

use crate::error::WickError;

/// 6-decimal fixed point scale — matches the working venue price exponent.
pub const SCALE: u128 = 1_000_000;
/// Basis points denominator (10_000 = 100%).
pub const BPS_DENOM: u128 = 10_000;
/// Starting staleness bound, in slots (~10s @ 400ms/slot). Tune per venue.
pub const MAX_TICK_AGE_SLOTS: u64 = 25;

/// The venue authority regime that gates whether the guard may act alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityRequirement {
    /// The guard may construct + invoke the venue action immediately (§8.4).
    Autonomous,
    /// The guard may only build the owner-signed instruction (never sign).
    CoSigned,
}

/// Every distinct action the guard can emit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionType {
    TopUp,
    PartialClose,
    TakeProfit,
}

/// A resolved action ready to dispatch (venue-agnostic in Phase 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    TopUp { amount: u128 },
    PartialClose { fraction_bps: u128 },
    TakeProfit,
    EscalateManualReview,
}

/// Per-action USD cap plus a daily total cap, enforced in §8.2.
#[derive(Clone, Copy, Debug)]
pub struct ActionCaps {
    pub top_up_usd_per_action: u128,
    pub partial_close_usd_per_action: u128,
    pub daily_total_usd: u128,
}

impl ActionCaps {
    #[inline]
    pub fn within_cap(&self, kind: ActionType, usd: u128) -> bool {
        match kind {
            ActionType::TopUp => usd <= self.top_up_usd_per_action && usd <= self.daily_total_usd,
            ActionType::PartialClose => {
                usd <= self.partial_close_usd_per_action && usd <= self.daily_total_usd
            }
            ActionType::TakeProfit => true,
        }
    }
}

/// Read snapshot of a watched position.
#[derive(Clone, Copy, Debug)]
pub struct HealthSnapshot {
    pub collateral: u128,
    pub size: i128,
    pub entry: u128,
    pub current_price: u128,
}

/// The immutable part of a position's protection policy.
#[derive(Clone, Copy, Debug)]
pub struct VenuePolicy {
    pub maintenance_bps: u128,
    /// Target buffer, in bps above 1.0, after a partial close.
    pub trigger_buffer_bps: u128,
    /// Fee on notional closed, in bps (venue fee schedule).
    pub fee_bps: u128,
    pub authority: AuthorityRequirement,
    pub caps: ActionCaps,
    pub take_profit: Option<u128>,
}

// -------------------------------------------------------------------------
// §8.1 Fixed-point health engine
// -------------------------------------------------------------------------

/// PnL for `size` (signed units) at `entry` vs `current`. Shorted positions are
/// handled for free by signed `size` — do not special-case them.
pub fn compute_pnl(
    size: i128,
    entry: u128,
    current: u128,
) -> Result<i128, WickError> {
    let entry = i128::try_from(entry).or(Err(WickError::MathOverflow))?;
    let current = i128::try_from(current).or(Err(WickError::MathOverflow))?;
    let price_delta = current
        .checked_sub(entry)
        .ok_or(WickError::MathOverflow)?;
    let raw = size.checked_mul(price_delta).ok_or(WickError::MathOverflow)?;
    raw.checked_div(SCALE as i128).ok_or(WickError::MathOverflow)
}

/// Maintenance margin required on `abs_size`, scaled by `margin_bps`.
pub fn compute_margin_required(abs_size: u128, margin_bps: u128) -> Result<u128, WickError> {
    let num = (abs_size as i128).checked_mul(margin_bps as i128)
        .ok_or(WickError::MathOverflow)?;
    (num / BPS_DENOM as i128).try_into().or(Err(WickError::MathOverflow))
}

/// Equity = collateral + pnl. Can be negative (insolvency).
pub fn equity(collateral: u128, pnl: i128) -> i128 {
    (collateral as i128).saturating_add(pnl)
}

/// Cross-multiplied breach test: `equity < margin_required`. Never divide.
pub fn is_liquidatable(
    collateral: u128,
    size: i128,
    entry: u128,
    current: u128,
    margin_bps: u128,
) -> Result<bool, WickError> {
    let pnl = compute_pnl(size, entry, current)?;
    let eq = equity(collateral, pnl);
    let req = compute_margin_required(size.unsigned_abs(), margin_bps)?;
    Ok(eq < (req as i128))
}

/// §8.1.3 Staleness bound for an incoming tick. `current >= last` expected.
pub fn accept_tick(current_slot: u64, last_check_slot: u64) -> bool {
    current_slot.saturating_sub(last_check_slot) <= MAX_TICK_AGE_SLOTS
}

/// §8.3 Bounded binary-search partial-close fraction in [0, BPS_DENOM].
///
/// Returns the minimal `f_bps` such that closing that fraction restores the
/// target buffer, or `CannotReachSafeBuffer` if even a full close can't.
pub fn solve_partial_close_fraction(
    collateral: u128,
    size: i128,
    entry: u128,
    current: u128,
    margin_bps: u128,
    buffer_bps: u128,
    fee_bps: u128,
) -> Result<u128, WickError> {
    let pnl = compute_pnl(size, entry, current)?;
    let m_full = compute_margin_required(size.unsigned_abs(), margin_bps)?;
    let target_full_num = (m_full as i128)
        .checked_mul(BPS_DENOM.saturating_add(buffer_bps) as i128)
        .ok_or(WickError::MathOverflow)?;
    let target_full: u128 = target_full_num
        .checked_div(BPS_DENOM as i128)
        .ok_or(WickError::MathOverflow)?
        .try_into()
        .or(Err(WickError::MathOverflow))?;

    let notional: i128 = (size.unsigned_abs() as i128)
        .checked_mul(current as i128)
        .ok_or(WickError::MathOverflow)?
        .checked_div(SCALE as i128)
        .ok_or(WickError::MathOverflow)?;
    let fee_full: i128 = notional
        .checked_mul(fee_bps as i128)
        .ok_or(WickError::MathOverflow)?
        .checked_div(BPS_DENOM as i128)
        .ok_or(WickError::MathOverflow)?;

    let is_safe = |f_bps: u128| -> Result<(bool, i128), WickError> {
        let rem = BPS_DENOM.saturating_sub(f_bps);
        let fee = fee_full
            .checked_mul(f_bps as i128)
            .ok_or(WickError::MathOverflow)?
            .checked_div(BPS_DENOM as i128)
            .ok_or(WickError::MathOverflow)?;
        // Closing fraction f realizes pnl*f and keeps pnl*(1-f) unrealized, so the
        // equity after the close is collateral - fee + pnl, independent of f.
        let equity_after = (collateral as i128)
            .checked_sub(fee)
            .ok_or(WickError::MathOverflow)?
            .checked_add(pnl)
            .ok_or(WickError::MathOverflow)?;
        // The remaining position still needs target_full * (1-f) of buffer.
        let remaining_target = (target_full as i128)
            .checked_mul(rem as i128)
            .ok_or(WickError::MathOverflow)?
            .checked_div(BPS_DENOM as i128)
            .ok_or(WickError::MathOverflow)?;
        Ok((equity_after >= remaining_target, equity_after))
    };

    if let (false, equity_at_full) = is_safe(BPS_DENOM)? {
        // Closing 100% still doesn't reach target, or equity is negative (insolvency).
        if equity_at_full <= 0 {
            return Err(WickError::CannotReachSafeBuffer);
        }
    }

    let (mut lo, mut hi): (u128, u128) = (0, BPS_DENOM);
    for _ in 0..20 {
        let mid = (lo.saturating_add(hi)) / 2;
        let (safe, _) = is_safe(mid)?;
        if safe {
            hi = mid;
        } else {
            lo = mid.saturating_add(1);
        }
    }
    Ok(hi)
}

// -------------------------------------------------------------------------
// §8.2 Action selection — full precedence with cap enforcement
// -------------------------------------------------------------------------

pub fn select_action(
    health: &HealthSnapshot,
    policy: &VenuePolicy,
    nonce: u64,
    last_nonce: u64,
) -> Result<Option<Action>, WickError> {
    if nonce <= last_nonce {
        return Ok(None); // stale/replayed — hard reject, no partial credit
    }

    // 1. TP fires on price crossing alone — independent of health, so run it
    //    before the liquidity gate (a position can be profitable yet still need
    //    to lock in take-profit).
    if let Some(tp) = policy.take_profit {
        if health.current_price >= tp {
            return Ok(Some(Action::TakeProfit));
        }
    }

    if !is_liquidatable(
        health.collateral,
        health.size,
        health.entry,
        health.current_price,
        policy.maintenance_bps,
    )? {
        return Ok(None);
    }

    // 2. Top-up if it clears the breach on its own and is within cap.
    let pnl = compute_pnl(health.size, health.entry, health.current_price)?;
    let eq = equity(health.collateral, pnl);
    let req = compute_margin_required(health.size.unsigned_abs(), policy.maintenance_bps)? as i128;
    let deficit = req.saturating_sub(eq);
    if deficit > 0 {
        let needed = deficit as u128;
        if policy.caps.within_cap(ActionType::TopUp, needed) {
            return Ok(Some(Action::TopUp { amount: needed }));
        }
    }

    // 3. Partial close (solver). Only if top-up alone can't clear it within cap.
    let f_bps = match solve_partial_close_fraction(
        health.collateral,
        health.size,
        health.entry,
        health.current_price,
        policy.maintenance_bps,
        policy.trigger_buffer_bps,
        policy.fee_bps,
    ) {
        Ok(f) => f,
        Err(WickError::CannotReachSafeBuffer) => return Ok(Some(Action::EscalateManualReview)),
        Err(e) => return Err(e),
    };
    if policy.caps.within_cap(ActionType::PartialClose, f_bps) {
        return Ok(Some(Action::PartialClose { fraction_bps: f_bps }));
    }

    // 4. Nothing fits inside caps — escalate, never silently no-op.
    Ok(Some(Action::EscalateManualReview))
}

// -------------------------------------------------------------------------
// On-chain account layouts
// -------------------------------------------------------------------------

/// Singleton program config — kill-switch + global pause (Phase 1 view).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RouteConfig {
    /// Authority allowed to administer the config.
    pub authority: [u8; 32],
    /// Paused kill-switch (Section 7).
    pub paused: bool,
    /// Reserved for growth.
    pub _padding: [u8; 31],
}

/// A watched position + its policy.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PositionGuard {
    pub venue: u8,
    pub venue_owner: [u8; 32],
    pub co_authority: [u8; 32],
    pub authority_req: u8,
    pub policy: VenuePolicy,
    pub collateral: u128,
    pub size: i128,
    pub entry: u128,
    pub current_price: u128,
    pub nonce: u64,
    pub last_check_slot: u64,
    pub pending: Option<Action>,
}

/// Thin, capped, user-funded margin-pump reserve — a 2-of-2 wallet.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GuardMarginWallet {
    pub owner: [u8; 32],
    pub co_authority: [u8; 32],
    pub balance: u128,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pnl_long_and_short() {
        // Long 100 units @ $50, now $60 => +$1_000 (=$1_000_000_000 at 6dp scale)
        assert_eq!(compute_pnl(100 * SCALE as i128, 50 * SCALE as u128, 60 * SCALE as u128).unwrap(), 1_000_000_000);
        // Short -100 units @ $50, now $40 => +$1_000
        assert_eq!(compute_pnl(-100 * SCALE as i128, 50 * SCALE as u128, 40 * SCALE as u128).unwrap(), 1_000_000_000);
        // Short -100 @ $50, now $60 => -$1_000
        assert_eq!(compute_pnl(-100 * SCALE as i128, 50 * SCALE as u128, 60 * SCALE as u128).unwrap(), -1_000_000_000);
    }

    #[test]
    fn margin_required_5pct() {
        assert_eq!(compute_margin_required(100 * SCALE as u128, 500).unwrap(), 5 * SCALE as u128);
    }

    #[test]
    fn breach_detected_cross_multiplied() {
        // Long 100 @ 50, now 48; collateral 200. equity=200 + (100*-2)=0 -> breach
        assert!(is_liquidatable(200 * SCALE as u128, 100 * SCALE as i128, 50 * SCALE as u128, 48 * SCALE as u128, 500).unwrap());
        // Healthy: collateral 600, price 55 => equity 1100 >= req 500
        assert!(!is_liquidatable(600 * SCALE as u128, 100 * SCALE as i128, 50 * SCALE as u128, 55 * SCALE as u128, 500).unwrap());
    }

    #[test]
    fn accept_tick_bounds() {
        assert!(accept_tick(100, 80));       // 20 slots ok
        assert!(!accept_tick(100, 74));      // 26 too old
        assert!(accept_tick(100, 100));      // same slot ok
    }

    #[test]
    fn solver_reaches_safe_buffer() {
        // Not safe to do nothing (collateral 45m < target 52.5m) but full close
        // leaves positive equity (fee 0.1m + pnl 0), so a partial close restores it.
        let collateral = 45 * SCALE as u128;
        let size = 100 * SCALE as i128;
        let entry = 50 * SCALE as u128;
        let current = 50 * SCALE as u128;   // pnl = 0
        let margin_bps = 5000;              // req full = 50% * $100 = $50, target = $52.5
        let buffer_bps = 500;               // target = 1.05 * 50 = 52.5
        let fee_bps = 10;
        let f = solve_partial_close_fraction(collateral, size, entry, current, margin_bps, buffer_bps, fee_bps).unwrap();
        assert!(f > 0);
        assert!(f <= BPS_DENOM);
    }

    #[test]
    fn solver_escalates_when_unsalvageable() {
        // Equity after FULL close = collateral - fee + pnl is still negative,
        // so no fraction can restore the buffer => escalate.
        let collateral = 100 * SCALE as u128;
        let size = 1_000 * SCALE as i128;
        let entry = 50 * SCALE as u128;
        let current = 1 * SCALE as u128;      // pnl = -49,000m; collateral 100m can't cover the loss
        let margin_bps = 500;
        let buffer_bps = 500;
        let fee_bps = 0;
        let err = solve_partial_close_fraction(collateral, size, entry, current, margin_bps, buffer_bps, fee_bps);
        assert_eq!(err.unwrap_err(), WickError::CannotReachSafeBuffer);
    }

    #[test]
    fn action_selector_priorities() {
        let policy = VenuePolicy {
            maintenance_bps: 500,
            trigger_buffer_bps: 500,
            fee_bps: 10,
            authority: AuthorityRequirement::Autonomous,
            caps: ActionCaps {
                top_up_usd_per_action: u128::MAX,
                partial_close_usd_per_action: u128::MAX,
                daily_total_usd: u128::MAX,
            },
            take_profit: Some(60 * SCALE as u128),
        };
        // Breached and TP crossed -> TakeProfit wins.
        let h = HealthSnapshot {
            collateral: 50 * SCALE as u128,
            size: 100 * SCALE as i128,
            entry: 50 * SCALE as u128,
            current_price: 61 * SCALE as u128,
        };
        assert_eq!(select_action(&h, &policy, 5, 4).unwrap(), Some(Action::TakeProfit));

        // Replay nonce -> None even if breached.
        assert_eq!(select_action(&h, &policy, 4, 4).unwrap(), None);

        // Breached, TP not crossed, top-up within cap -> TopUp.
        let h2 = HealthSnapshot {
            collateral: 50 * SCALE as u128,
            size: 100 * SCALE as i128,
            entry: 50 * SCALE as u128,
            current_price: 49 * SCALE as u128,
        };
        // equity = 50 + (100 * -1) = -50 -> deficit = req(5) - (-50) = 55
        assert!(matches!(select_action(&h2, &policy, 5, 4).unwrap(), Some(Action::TopUp { .. })));

        // Over-cap -> escalate rather than silent no-op.
        let tight = VenuePolicy {
            caps: ActionCaps {
                top_up_usd_per_action: 1,
                partial_close_usd_per_action: 1,
                daily_total_usd: 1,
            },
            ..policy
        };
        assert_eq!(select_action(&h2, &tight, 5, 4).unwrap(), Some(Action::EscalateManualReview));
    }
}