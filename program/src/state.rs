//! On-chain state layouts for the Wick guard program.
//!
//! All math §8.1 is defined here so unit tests can exercise it without needing
//! a full account/instruction context.

use crate::error::WickError;

/// 6-decimal fixed point scale — matches the working venue price exponent.
pub const SCALE: u128 = 1_000_000;
/// Basis points denominator (10_000 = 100%).
pub const BPS_DENOM: u128 = 10_000;
/// Staleness bound between consecutive ticks, in seconds.
///
/// Deliberately wall-clock rather than slot-denominated. A slot is not a
/// portable unit of time: Solana devnet runs ~400ms/slot, but the MagicBlock
/// Ephemeral Rollup a delegated guard runs on (§8.6) runs ~50ms/slot. The old
/// 25-slot bound meant ~10s on the base layer and ~1.2s on the ER, so a
/// perfectly healthy 5s cranker cadence read as stale on every single ER tick
/// and the guard degraded permanently — observed on devnet before this change.
/// Seconds mean the same thing on both layers.
pub const MAX_TICK_AGE_SECS: i64 = 10;

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

/// A resolved action ready to dispatch (venue-agnostic).
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

/// Length of one daily spend epoch, in seconds.
///
/// Wall-clock for the same reason as `MAX_TICK_AGE_SECS`: slot length is ~400ms
/// on base Solana but ~50ms on the MagicBlock ER, so a slot-denominated "day"
/// would roll the daily budget over ~7 times per day on a delegated guard.
/// `clock.unix_timestamp` is trusted data here — `read_price_no_older_than`
/// already relies on it for the Pyth age check.
pub const DAILY_EPOCH_SECS: i64 = 86_400;

impl ActionCaps {
    /// Per-action ceiling only. The daily budget is a separate question — it
    /// depends on what has already been spent this epoch, which lives on the
    /// account, not in the policy. See `within_daily`.
    #[inline]
    pub fn within_cap(&self, kind: ActionType, usd: u128) -> bool {
        match kind {
            ActionType::TopUp => usd <= self.top_up_usd_per_action,
            ActionType::PartialClose => usd <= self.partial_close_usd_per_action,
            ActionType::TakeProfit => true,
        }
    }

    /// Whether `usd` still fits in the daily budget given `spent` so far.
    ///
    /// `daily_total_usd` used to be compared against a *single* action, which
    /// made it just a second per-action ceiling: N successive within-cap top-ups
    /// across N ticks could drain the margin wallet without ever tripping it.
    /// The budget only means anything against an accumulator.
    #[inline]
    pub fn within_daily(&self, spent: u128, usd: u128) -> bool {
        match spent.checked_add(usd) {
            Some(total) => total <= self.daily_total_usd,
            None => false,
        }
    }
}

/// Roll the daily accumulator over if the epoch has elapsed.
///
/// Returns the `(spent, epoch_start_ts)` pair to use for this tick. A guard
/// whose epoch has expired starts a fresh budget anchored at `now_ts`. A
/// timestamp before `epoch_start_ts` (a clock rewinding across an ER handover)
/// is treated as "still inside the epoch" rather than wrapping — `saturating_sub`
/// keeps it from resetting the budget early.
#[inline]
pub fn roll_daily_epoch(spent: u128, epoch_start_ts: i64, now_ts: i64) -> (u128, i64) {
    // An unset epoch start (0) adopts `now` without zeroing an accumulator that
    // is already 0 anyway, so the first spend of a guard's life starts a real
    // epoch instead of one anchored at the unix epoch.
    if epoch_start_ts <= 0 {
        return (spent, now_ts);
    }
    if now_ts.saturating_sub(epoch_start_ts) >= DAILY_EPOCH_SECS {
        (0, now_ts)
    } else {
        (spent, epoch_start_ts)
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
///
/// Rounding: `checked_div` truncates toward zero, so a loss rounds *up* toward
/// zero by up to 1 unit at 6dp — i.e. in the trader's favour, overstating equity
/// by at most $0.000001. This is deliberate and left as-is: the error is fixed at
/// one micro-dollar regardless of position size, which is many orders of
/// magnitude below the maintenance margin it feeds into, and below the price
/// granularity of any venue we route to. Rounding losses away from zero instead
/// would be marginally more conservative but adds a signed-division special case
/// to the hottest path in the engine for no practical gain.
pub fn compute_pnl(size: i128, entry: u128, current: u128) -> Result<i128, WickError> {
    let entry = i128::try_from(entry).or(Err(WickError::MathOverflow))?;
    let current = i128::try_from(current).or(Err(WickError::MathOverflow))?;
    let price_delta = current.checked_sub(entry).ok_or(WickError::MathOverflow)?;
    let raw = size
        .checked_mul(price_delta)
        .ok_or(WickError::MathOverflow)?;
    raw.checked_div(SCALE as i128)
        .ok_or(WickError::MathOverflow)
}

/// Maintenance margin required on `abs_size` at `current`, scaled by
/// `margin_bps`.
///
/// The basis is *notional* (`abs_size * current`), not the raw unit count.
/// Taking bps of a unit count yields units, which is then compared against
/// `equity` in USD — a dimensional mismatch that makes the requirement
/// price-independent and collapses it by a factor of `current`. The practical
/// effect is a guard that only fires at outright insolvency instead of at the
/// maintenance threshold, which is the whole point of the engine.
pub fn compute_margin_required(
    abs_size: u128,
    margin_bps: u128,
    current: u128,
) -> Result<u128, WickError> {
    let notional = abs_size
        .checked_mul(current)
        .ok_or(WickError::MathOverflow)?
        .checked_div(SCALE)
        .ok_or(WickError::MathOverflow)?;
    notional
        .checked_mul(margin_bps)
        .ok_or(WickError::MathOverflow)?
        .checked_div(BPS_DENOM)
        .ok_or(WickError::MathOverflow)
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
    let req = compute_margin_required(size.unsigned_abs(), margin_bps, current)?;
    Ok(eq < (req as i128))
}

/// §8.1.3 Staleness bound for an incoming tick. `now >= last` expected.
///
/// A first tick (`last == 0`, never checked) is fresh: there is no previous tick
/// to be stale relative to, and treating the unix epoch as the last check would
/// degrade every brand-new guard. A clock that moved backwards (`now < last`,
/// possible across an ER handover) is also treated as fresh rather than stale —
/// it says nothing about the price feed, and `saturating_sub` would otherwise
/// read a backwards jump as age 0 anyway.
pub fn accept_tick(now_ts: i64, last_check_ts: i64) -> bool {
    if last_check_ts <= 0 {
        return true;
    }
    now_ts.saturating_sub(last_check_ts) <= MAX_TICK_AGE_SECS
}

/// §8.1.3 Consecutive stale ticks before the guard flips to `degraded`.
/// Policy-configurable later; start at 3 as the doc specifies.
pub const MAX_STALE_STREAK: u8 = 3;

/// §8.1.3 Stale-tick state machine.
///
/// A rejected tick cannot just mean "no-op this tick" — several stale ticks in
/// a row mean the guard is silently protecting against dead data. On N
/// consecutive stale ticks the guard flips to `degraded`, which the frontend
/// must surface immediately (not just log). A fresh tick clears the streak and
/// the degraded flag.
///
/// Returns `(new_streak, degraded)`.
pub fn track_tick_freshness(stale_streak: u8, fresh: bool) -> (u8, bool) {
    if fresh {
        (0, false)
    } else {
        let streak = stale_streak.saturating_add(1);
        (streak, streak >= MAX_STALE_STREAK)
    }
}

// -------------------------------------------------------------------------
// §8.3 Venue reconciliation
// -------------------------------------------------------------------------

/// How the guard's model of the position compares to the venue's own bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconcileStatus {
    /// No reconciliation has ever run. The model is whatever the owner enrolled.
    NeverReconciled,
    /// The venue agrees with the guard's model, within tolerance.
    Converged,
    /// The venue disagrees beyond tolerance. Autonomous execution is disarmed
    /// until an owner `UpdatePosition` resolves the model.
    Diverged,
}

impl ReconcileStatus {
    pub fn to_byte(self) -> u8 {
        match self {
            ReconcileStatus::NeverReconciled => 0,
            ReconcileStatus::Converged => 1,
            ReconcileStatus::Diverged => 2,
        }
    }

    pub fn from_byte(b: u8) -> Result<Self, WickError> {
        match b {
            0 => Ok(ReconcileStatus::NeverReconciled),
            1 => Ok(ReconcileStatus::Converged),
            2 => Ok(ReconcileStatus::Diverged),
            _ => Err(WickError::InvalidInstruction),
        }
    }
}

/// Size divergence the guard will absorb without disarming, in basis points of
/// the venue-reported size.
///
/// Not zero, because zero would disarm the guard constantly for reasons that are
/// not divergence: a perp position accrues funding and settles PnL in base
/// precision, and a reduce order can fill a few base units short of its target.
/// 25 bps (0.25%) is well inside the smallest maintenance buffer the policy
/// allows, so a divergence this small cannot change a liquidation decision — and
/// well below any real resize, partial close, or re-open, which move whole
/// percentage points. A model wrong by more than this is a model the guard must
/// not size an order from.
pub const RECONCILE_TOLERANCE_BPS: u128 = 25;

/// Compare the guard's model of the position against the venue's own report.
///
/// Pure and total, so the fail-closed rule is unit-testable rather than
/// entangled with account plumbing. Any comparison that cannot be computed
/// resolves to `Diverged`: the guard's whole reason to disarm is uncertainty
/// about the size it would trade.
pub fn reconcile_verdict(model_size: i128, venue_size: i128) -> ReconcileStatus {
    if model_size == venue_size {
        return ReconcileStatus::Converged;
    }
    // A sign flip is a different position, never a rounding artefact — a guard
    // that thinks it is long while the venue has it short would reduce in the
    // wrong direction and *increase* exposure.
    if (model_size < 0) != (venue_size < 0) {
        return ReconcileStatus::Diverged;
    }
    // Flat at the venue while the guard models exposure (or the reverse) has no
    // meaningful ratio to test against; it is the closed-behind-our-back case.
    if venue_size == 0 || model_size == 0 {
        return ReconcileStatus::Diverged;
    }
    let venue_abs = venue_size.unsigned_abs();
    let delta = model_size.unsigned_abs().abs_diff(venue_abs);
    let Some(scaled) = delta.checked_mul(BPS_DENOM) else {
        return ReconcileStatus::Diverged;
    };
    let Some(allowed) = venue_abs.checked_mul(RECONCILE_TOLERANCE_BPS) else {
        return ReconcileStatus::Diverged;
    };
    if scaled <= allowed {
        ReconcileStatus::Converged
    } else {
        ReconcileStatus::Diverged
    }
}

/// §8.4 Two-regime authority dispatch result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchRegime {
    /// Execute the venue action now; the nonce is committed immediately.
    Autonomous,
    /// Build the owner-signed instruction and hold it as pending; the nonce
    /// must NOT advance until the owner co-signs on L1 (§8.4). Advancing it at
    /// build-time would treat a second genuine breach (arriving while the owner
    /// reads) as a replay and silently drop it.
    CoSigned,
}

/// §8.4 — decide the dispatch regime for a guard and the nonce that a *successful*
/// autonomous execution would commit.
///
/// The nonce commit rule lives here so it is unit-testable: only the Autonomous
/// branch may advance `nonce`; the CoSigned branch defers it to the confirm step.
pub fn guard_act(
    policy: &VenuePolicy,
    tick_nonce: u64,
) -> Result<(DispatchRegime, u64), WickError> {
    match policy.authority {
        AuthorityRequirement::Autonomous => Ok((DispatchRegime::Autonomous, tick_nonce)),
        AuthorityRequirement::CoSigned => Ok((DispatchRegime::CoSigned, tick_nonce)),
    }
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
    let m_full = compute_margin_required(size.unsigned_abs(), margin_bps, current)?;
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

/// The USD (6dp) a given action commits against the daily budget.
///
/// Take-profit and escalation commit nothing: a TP closes the position at the
/// owner's own target (it spends no margin and is never something the guard
/// should be rate-limited out of), and escalation is a notification.
pub fn action_daily_usd(action: Action, notional: u128) -> u128 {
    match action {
        Action::TopUp { amount } => amount,
        Action::PartialClose { fraction_bps } => notional
            .saturating_mul(fraction_bps)
            .checked_div(BPS_DENOM)
            .unwrap_or(u128::MAX),
        Action::TakeProfit | Action::EscalateManualReview => 0,
    }
}

pub fn select_action(
    health: &HealthSnapshot,
    policy: &VenuePolicy,
    nonce: u64,
    last_nonce: u64,
    daily_spent_usd: u128,
) -> Result<Option<Action>, WickError> {
    if nonce <= last_nonce {
        return Ok(None); // stale/replayed — hard reject, no partial credit
    }

    // 1. TP fires on price crossing alone — independent of health, so run it
    //    before the liquidity gate (a position can be profitable yet still need
    //    to lock in take-profit).
    //
    //    The crossing direction follows the sign of `size`: a long takes profit
    //    above its target, a short below it. Testing `current >= tp` for both
    //    fires a short's take-profit on the way up — precisely when the short is
    //    losing — and, since a short's target sits below entry, on the very
    //    first tick after enrollment.
    if let Some(tp) = policy.take_profit {
        let crossed = match health.size.signum() {
            1 => health.current_price >= tp,
            -1 => health.current_price <= tp,
            _ => false, // no position to take profit on
        };
        if crossed {
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
    let req = compute_margin_required(
        health.size.unsigned_abs(),
        policy.maintenance_bps,
        health.current_price,
    )? as i128;
    // Target the trigger buffer, not bare maintenance. Restoring equity to
    // exactly `req` lands it on the liquidation line, so any adverse tick
    // re-breaches and each re-breach burns another top-up against the daily
    // cap. This mirrors the partial-close solver, which targets
    // `req * (1 + buffer_bps)` (§8.2).
    let target = (req as u128)
        .checked_mul(BPS_DENOM.saturating_add(policy.trigger_buffer_bps))
        .ok_or(WickError::MathOverflow)?
        .checked_div(BPS_DENOM)
        .ok_or(WickError::MathOverflow)?;
    let deficit = (target as i128).saturating_sub(eq);
    if deficit > 0 {
        let needed = deficit as u128;
        if policy.caps.within_cap(ActionType::TopUp, needed)
            && policy.caps.within_daily(daily_spent_usd, needed)
        {
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
    // The cap is denominated in USD, so it must be applied to the USD notional
    // actually closed — not to `f_bps`, which is a fraction in [0, BPS_DENOM].
    // Comparing the fraction against a USD ceiling made the cap inert: every
    // possible value passed, so step 4 below was unreachable via this branch and
    // an unbounded close executed where the guard should have escalated.
    let notional = health
        .size
        .unsigned_abs()
        .checked_mul(health.current_price)
        .ok_or(WickError::MathOverflow)?
        .checked_div(SCALE)
        .ok_or(WickError::MathOverflow)?;
    let closed_usd = notional
        .checked_mul(f_bps)
        .ok_or(WickError::MathOverflow)?
        .checked_div(BPS_DENOM)
        .ok_or(WickError::MathOverflow)?;
    if policy.caps.within_cap(ActionType::PartialClose, closed_usd)
        && policy.caps.within_daily(daily_spent_usd, closed_usd)
    {
        return Ok(Some(Action::PartialClose {
            fraction_bps: f_bps,
        }));
    }

    // 4. Nothing fits inside caps — escalate, never silently no-op.
    Ok(Some(Action::EscalateManualReview))
}

// -------------------------------------------------------------------------
// On-chain account layouts
// -------------------------------------------------------------------------

/// Singleton program config — kill-switch + global pause.
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
    /// Wall-clock seconds of the last accepted tick, not a slot — see
    /// `MAX_TICK_AGE_SECS`.
    pub last_check_ts: i64,
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
        assert_eq!(
            compute_pnl(100 * SCALE as i128, 50 * SCALE, 60 * SCALE).unwrap(),
            1_000_000_000
        );
        // Short -100 units @ $50, now $40 => +$1_000
        assert_eq!(
            compute_pnl(-100 * SCALE as i128, 50 * SCALE, 40 * SCALE).unwrap(),
            1_000_000_000
        );
        // Short -100 @ $50, now $60 => -$1_000
        assert_eq!(
            compute_pnl(-100 * SCALE as i128, 50 * SCALE, 60 * SCALE).unwrap(),
            -1_000_000_000
        );
    }

    #[test]
    fn margin_required_is_5pct_of_notional() {
        // 100 units @ $50 = $5000 notional; 5% of that is $250.
        assert_eq!(
            compute_margin_required(100 * SCALE, 500, 50 * SCALE).unwrap(),
            250 * SCALE
        );
    }

    #[test]
    fn margin_required_tracks_price() {
        // The requirement is a function of notional, so it must move with price.
        // A unit-count basis would return the same number for both.
        let cheap = compute_margin_required(10 * SCALE, 500, 10 * SCALE).unwrap();
        let dear = compute_margin_required(10 * SCALE, 500, 200 * SCALE).unwrap();
        // 10 @ $10 = $100 notional -> $5; 10 @ $200 = $2000 notional -> $100.
        assert_eq!(cheap, 5 * SCALE);
        assert_eq!(dear, 100 * SCALE);
        assert!(dear > cheap);
    }

    #[test]
    fn breach_detected_cross_multiplied() {
        // Long 100 @ 50, now 48; collateral 200. equity=200 + (100*-2)=0,
        // required = 100*48*5% = 240 -> breach
        assert!(is_liquidatable(
            200 * SCALE,
            100 * SCALE as i128,
            50 * SCALE,
            48 * SCALE,
            500
        )
        .unwrap());
        // Healthy: collateral 600, price 55 => equity 1100 >= req 275
        assert!(!is_liquidatable(
            600 * SCALE,
            100 * SCALE as i128,
            50 * SCALE,
            55 * SCALE,
            500
        )
        .unwrap());
    }

    #[test]
    fn leveraged_breach_is_caught_at_maintenance_not_insolvency() {
        // Regression: a unit-count margin basis made the requirement
        // price-independent ($5 here instead of $740), so a position sitting
        // $60 above maintenance on 15x leverage read as comfortably healthy and
        // the guard stayed silent until equity hit zero.
        let collateral = 1_000 * SCALE;
        let size = 100 * SCALE as i128;
        let entry = 150 * SCALE;

        // equity = 1000 + 100*(148-150) = 800; required = 100*148*5% = 740.
        // Above maintenance, but inside the trigger buffer.
        assert!(!is_liquidatable(collateral, size, entry, 148 * SCALE, 500).unwrap());
        assert_eq!(
            compute_margin_required(size.unsigned_abs(), 500, 148 * SCALE).unwrap(),
            740 * SCALE
        );

        // equity = 1000 + 100*(147-150) = 700 < required 735 -> breach, while
        // the position is still far from insolvent.
        assert!(is_liquidatable(collateral, size, entry, 147 * SCALE, 500).unwrap());
    }

    #[test]
    fn accept_tick_bounds() {
        assert!(accept_tick(100, 90)); // 10s — exactly the bound, ok
        assert!(!accept_tick(100, 89)); // 11s too old
        assert!(accept_tick(100, 100)); // same second ok
    }

    /// Regression: the bound used to be 25 *slots*. On the MagicBlock ER a slot
    /// is ~50ms, so 25 slots is ~1.2s and the cranker's 5s cadence read as stale
    /// on every tick — the guard degraded permanently on devnet. Seconds are
    /// layer-independent, so the same cadence is fresh on both.
    #[test]
    fn er_cadence_is_fresh_at_both_slot_rates() {
        let cadence_secs = 5;
        // Base layer: 5s is ~12 slots at 400ms. ER: 5s is ~100 slots at 50ms.
        // Neither number appears here any more, which is the point.
        assert!(accept_tick(1_700_000_000 + cadence_secs, 1_700_000_000));
    }

    #[test]
    fn stale_streak_degrades_after_three_and_recovers() {
        // Fresh tick clears the streak and degraded flag.
        assert_eq!(track_tick_freshness(0, true), (0, false));
        assert_eq!(track_tick_freshness(2, true), (0, false));

        // Stale ticks build the streak; the third flips degraded.
        assert_eq!(track_tick_freshness(0, false), (1, false));
        assert_eq!(track_tick_freshness(1, false), (2, false));
        assert_eq!(track_tick_freshness(2, false), (3, true));

        // Streak saturates — stays degraded.
        assert_eq!(track_tick_freshness(3, false), (4, true));
    }

    #[test]
    fn guard_act_commits_nonce_only_on_autonomous() {
        let base = VenuePolicy {
            maintenance_bps: 0,
            trigger_buffer_bps: 0,
            fee_bps: 0,
            authority: AuthorityRequirement::Autonomous,
            caps: ActionCaps {
                top_up_usd_per_action: 0,
                partial_close_usd_per_action: 0,
                daily_total_usd: 0,
            },
            take_profit: None,
        };
        let auth_auto = base;
        let auth_co = VenuePolicy {
            authority: AuthorityRequirement::CoSigned,
            ..base
        };

        // Autonomous: execute now, nonce = tick_nonce.
        let (regime, expected) = guard_act(&auth_auto, 7).unwrap();
        assert_eq!(regime, DispatchRegime::Autonomous);
        assert_eq!(expected, 7);

        // CoSigned: defer, nonce = tick_nonce but must NOT be committed by the
        // build step (§8.4) — the confirm step commits it later.
        let (regime, expected) = guard_act(&auth_co, 7).unwrap();
        assert_eq!(regime, DispatchRegime::CoSigned);
        assert_eq!(expected, 7);
    }

    #[test]
    fn solver_reaches_safe_buffer() {
        // Not safe to do nothing (collateral 45m < target 52.5m) but full close
        // leaves positive equity (fee 0.1m + pnl 0), so a partial close restores it.
        let collateral = 45 * SCALE;
        let size = 100 * SCALE as i128;
        let entry = 50 * SCALE;
        let current = 50 * SCALE; // pnl = 0
        let margin_bps = 5000; // req full = 50% * $100 = $50, target = $52.5
        let buffer_bps = 500; // target = 1.05 * 50 = 52.5
        let fee_bps = 10;
        let f = solve_partial_close_fraction(
            collateral, size, entry, current, margin_bps, buffer_bps, fee_bps,
        )
        .unwrap();
        assert!(f > 0);
        assert!(f <= BPS_DENOM);
    }

    #[test]
    fn solver_escalates_when_unsalvageable() {
        // Equity after FULL close = collateral - fee + pnl is still negative,
        // so no fraction can restore the buffer => escalate.
        let collateral = 100 * SCALE;
        let size = 1_000 * SCALE as i128;
        let entry = 50 * SCALE;
        let current = SCALE; // pnl = -49,000m; collateral 100m can't cover the loss
        let margin_bps = 500;
        let buffer_bps = 500;
        let fee_bps = 0;
        let err = solve_partial_close_fraction(
            collateral, size, entry, current, margin_bps, buffer_bps, fee_bps,
        );
        assert_eq!(err.unwrap_err(), WickError::CannotReachSafeBuffer);
    }

    #[test]
    fn short_take_profit_fires_below_entry_not_above() {
        // A short's take-profit sits *below* entry. Testing `current >= tp` for
        // every position (the pre-fix behaviour) fired a short's TP on the way
        // up — exactly when it is losing — and, because the target is below
        // entry, on the very first tick after enrollment.
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
            take_profit: Some(40 * SCALE), // short entered at 50, targets 40
        };
        let short_at = |current: u128| HealthSnapshot {
            collateral: 5_000 * SCALE, // amply collateralized: isolates the TP path
            size: -(100 * SCALE as i128),
            entry: 50 * SCALE,
            current_price: current,
        };

        // Regression: the very first tick after enrollment, price still at entry.
        // Pre-fix this returned TakeProfit and closed the position instantly.
        assert_eq!(
            select_action(&short_at(50 * SCALE), &policy, 5, 4, 0).unwrap(),
            None
        );

        // Moving up = the short is losing. Must not take profit.
        assert_eq!(
            select_action(&short_at(55 * SCALE), &policy, 5, 4, 0).unwrap(),
            None
        );

        // Reaching the target from above fires.
        assert_eq!(
            select_action(&short_at(40 * SCALE), &policy, 5, 4, 0).unwrap(),
            Some(Action::TakeProfit)
        );
        // And below it.
        assert_eq!(
            select_action(&short_at(39 * SCALE), &policy, 5, 4, 0).unwrap(),
            Some(Action::TakeProfit)
        );

        // The long path still keys off the opposite side of the target.
        let long_at_39 = HealthSnapshot {
            size: 100 * SCALE as i128,
            ..short_at(39 * SCALE)
        };
        assert_eq!(select_action(&long_at_39, &policy, 5, 4, 0).unwrap(), None);
        let long_at_41 = HealthSnapshot {
            size: 100 * SCALE as i128,
            ..short_at(41 * SCALE)
        };
        assert_eq!(
            select_action(&long_at_41, &policy, 5, 4, 0).unwrap(),
            Some(Action::TakeProfit)
        );
    }

    #[test]
    fn zero_size_never_takes_profit() {
        // There is no position to close, so no crossing can fire — in either
        // direction. `signum() == 0` must fall through, not pick a side.
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
            take_profit: Some(50 * SCALE),
        };
        let flat = |current: u128| HealthSnapshot {
            collateral: 100 * SCALE,
            size: 0,
            entry: 50 * SCALE,
            current_price: current,
        };
        assert_eq!(
            select_action(&flat(60 * SCALE), &policy, 5, 4, 0).unwrap(),
            None
        );
        assert_eq!(
            select_action(&flat(50 * SCALE), &policy, 5, 4, 0).unwrap(),
            None
        );
        assert_eq!(
            select_action(&flat(40 * SCALE), &policy, 5, 4, 0).unwrap(),
            None
        );
    }

    #[test]
    fn top_up_restores_the_buffer_not_just_maintenance() {
        // Topping up to bare `req` lands equity exactly on the liquidation line,
        // so the next adverse tick re-breaches and burns another top-up against
        // the daily cap. The top-up must clear the same target the close solver
        // aims for: req * (1 + buffer_bps).
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
            take_profit: None,
        };
        // Long 100 @ 50, now 49: pnl = -100, equity = -50.
        // req = 100 * 49 * 5% = 245; target = 245 * 1.05 = 257.25.
        let h = HealthSnapshot {
            collateral: 50 * SCALE,
            size: 100 * SCALE as i128,
            entry: 50 * SCALE,
            current_price: 49 * SCALE,
        };
        let Some(Action::TopUp { amount }) = select_action(&h, &policy, 5, 4, 0).unwrap() else {
            panic!("expected a top-up");
        };
        assert_eq!(amount, 307_250_000); // 257.25 - (-50)

        // Applying it leaves the position above maintenance, with the buffer
        // intact — not sitting on the line.
        let after = h.collateral + amount;
        assert!(!is_liquidatable(after, h.size, h.entry, h.current_price, 500).unwrap());
        let eq_after = equity(
            after,
            compute_pnl(h.size, h.entry, h.current_price).unwrap(),
        );
        assert_eq!(eq_after, 257_250_000);
    }

    #[test]
    fn partial_close_cap_binds_on_usd_notional() {
        // Regression: the cap was compared against `f_bps` (a fraction in
        // [0, 10_000]) rather than the USD notional closed. Since caps are USD at
        // 6dp, every fraction passed and the cap never bound — so an unbounded
        // close executed where the guard should have escalated.
        let base = VenuePolicy {
            maintenance_bps: 5_000,
            trigger_buffer_bps: 500,
            fee_bps: 10,
            authority: AuthorityRequirement::Autonomous,
            caps: ActionCaps {
                top_up_usd_per_action: 0, // force the ladder past the top-up branch
                partial_close_usd_per_action: u128::MAX,
                daily_total_usd: u128::MAX,
            },
            take_profit: None,
        };
        // Long 100 @ 50, now 50: notional = $5_000, pnl = 0, equity = $45,
        // req = $2_500 -> breached, and a partial close can restore it.
        let h = HealthSnapshot {
            collateral: 45 * SCALE,
            size: 100 * SCALE as i128,
            entry: 50 * SCALE,
            current_price: 50 * SCALE,
        };
        let Some(Action::PartialClose { fraction_bps }) =
            select_action(&h, &base, 5, 4, 0).unwrap()
        else {
            panic!("expected a partial close under an unbounded cap");
        };
        let closed_usd = (5_000 * SCALE) * fraction_bps / BPS_DENOM;

        // A cap just above the USD actually closed still admits the action.
        let generous = VenuePolicy {
            caps: ActionCaps {
                partial_close_usd_per_action: closed_usd + 1,
                ..base.caps
            },
            ..base
        };
        assert_eq!(
            select_action(&h, &generous, 5, 4, 0).unwrap(),
            Some(Action::PartialClose { fraction_bps })
        );

        // A cap just below it must escalate rather than close unbounded. Under
        // the old comparison this cap (~$4_949 at 6dp) sat far above any
        // possible `f_bps`, so it silently passed.
        let tight = VenuePolicy {
            caps: ActionCaps {
                partial_close_usd_per_action: closed_usd - 1,
                ..base.caps
            },
            ..base
        };
        assert_eq!(
            select_action(&h, &tight, 5, 4, 0).unwrap(),
            Some(Action::EscalateManualReview)
        );
    }

    #[test]
    fn daily_budget_blocks_the_multi_tick_drain() {
        // The attack §2.2 describes: `daily_total_usd` was compared against a
        // *single* action, so it was just a second per-action ceiling. N
        // successive within-cap top-ups across N ticks drained the margin
        // wallet without ever tripping it. Against an accumulator, the budget
        // has to run out.
        let policy = VenuePolicy {
            maintenance_bps: 500,
            trigger_buffer_bps: 500,
            fee_bps: 10,
            authority: AuthorityRequirement::Autonomous,
            caps: ActionCaps {
                top_up_usd_per_action: 500 * SCALE,
                partial_close_usd_per_action: 0, // force escalation once top-ups stop
                daily_total_usd: 1_000 * SCALE,
            },
            take_profit: None,
        };
        let h = HealthSnapshot {
            collateral: 50 * SCALE,
            size: 100 * SCALE as i128,
            entry: 50 * SCALE,
            current_price: 49 * SCALE,
        };
        // Each individual top-up is $307.25 — comfortably inside the $500
        // per-action cap, which is exactly why the per-action check alone never
        // stopped the drain.
        let Some(Action::TopUp { amount }) = select_action(&h, &policy, 5, 4, 0).unwrap() else {
            panic!("expected a top-up on an unspent budget");
        };
        assert_eq!(amount, 307_250_000);
        assert!(policy.caps.within_cap(ActionType::TopUp, amount));

        // Two more fit in the $1_000 budget; the third does not.
        assert!(select_action(&h, &policy, 5, 4, amount).unwrap().is_some());
        assert!(select_action(&h, &policy, 5, 4, amount * 2)
            .unwrap()
            .is_some());
        assert_eq!(
            select_action(&h, &policy, 5, 4, amount * 3).unwrap(),
            Some(Action::EscalateManualReview),
            "a spent budget must escalate, not keep topping up"
        );

        // And the guard escalates rather than silently no-oping — the breach is
        // still live, it just needs a human now.
        assert_eq!(
            select_action(&h, &policy, 5, 4, 1_000 * SCALE).unwrap(),
            Some(Action::EscalateManualReview)
        );
    }

    #[test]
    fn daily_budget_charges_partial_close_on_usd_not_fraction() {
        let caps = ActionCaps {
            top_up_usd_per_action: u128::MAX,
            partial_close_usd_per_action: u128::MAX,
            daily_total_usd: 10_000 * SCALE,
        };
        // A half close of a $5_000 position commits $2_500 — not "5000" (the
        // raw bps figure), which would understate the charge by ~1000x.
        assert_eq!(
            action_daily_usd(
                Action::PartialClose {
                    fraction_bps: 5_000
                },
                5_000 * SCALE
            ),
            2_500 * SCALE
        );
        assert_eq!(
            action_daily_usd(Action::TopUp { amount: 42 * SCALE }, 5_000 * SCALE),
            42 * SCALE
        );
        // Take-profit closes at the owner's own target and spends no margin;
        // escalation is a notification. Neither draws down the budget.
        assert_eq!(action_daily_usd(Action::TakeProfit, 5_000 * SCALE), 0);
        assert_eq!(
            action_daily_usd(Action::EscalateManualReview, 5_000 * SCALE),
            0
        );

        assert!(caps.within_daily(9_000 * SCALE, 1_000 * SCALE));
        assert!(!caps.within_daily(9_000 * SCALE, 1_000 * SCALE + 1));
        // A saturating add must not wrap into "allowed".
        assert!(!caps.within_daily(u128::MAX, 1));
    }

    #[test]
    fn daily_epoch_rolls_over_and_holds() {
        // Inside the epoch: the accumulator is preserved.
        assert_eq!(roll_daily_epoch(500, 1_000, 1_000), (500, 1_000));
        assert_eq!(
            roll_daily_epoch(500, 1_000, 1_000 + DAILY_EPOCH_SECS - 1),
            (500, 1_000)
        );
        // At the boundary: reset, re-anchored to now.
        assert_eq!(
            roll_daily_epoch(500, 1_000, 1_000 + DAILY_EPOCH_SECS),
            (0, 1_000 + DAILY_EPOCH_SECS)
        );
        // A fresh guard (epoch 0) adopts the current timestamp as its anchor
        // rather than carrying the zero forward — otherwise every first tick
        // measures its age from 1970 and rolls immediately.
        assert_eq!(roll_daily_epoch(0, 0, 1_700_000_000), (0, 1_700_000_000));
        // A rewound clock must not reset the budget early: saturating_sub keeps
        // it inside the epoch rather than wrapping to a huge elapsed count.
        assert_eq!(roll_daily_epoch(500, 1_000, 900), (500, 1_000));
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
            take_profit: Some(60 * SCALE),
        };
        // Breached and TP crossed -> TakeProfit wins.
        let h = HealthSnapshot {
            collateral: 50 * SCALE,
            size: 100 * SCALE as i128,
            entry: 50 * SCALE,
            current_price: 61 * SCALE,
        };
        assert_eq!(
            select_action(&h, &policy, 5, 4, 0).unwrap(),
            Some(Action::TakeProfit)
        );

        // Replay nonce -> None even if breached.
        assert_eq!(select_action(&h, &policy, 4, 4, 0).unwrap(), None);

        // Breached, TP not crossed, top-up within cap -> TopUp.
        let h2 = HealthSnapshot {
            collateral: 50 * SCALE,
            size: 100 * SCALE as i128,
            entry: 50 * SCALE,
            current_price: 49 * SCALE,
        };
        // equity = 50 + (100 * -1) = -50 -> deficit = req(5) - (-50) = 55
        assert!(matches!(
            select_action(&h2, &policy, 5, 4, 0).unwrap(),
            Some(Action::TopUp { .. })
        ));

        // Over-cap -> escalate rather than silent no-op.
        let tight = VenuePolicy {
            caps: ActionCaps {
                top_up_usd_per_action: 1,
                partial_close_usd_per_action: 1,
                daily_total_usd: 1,
            },
            ..policy
        };
        assert_eq!(
            select_action(&h2, &tight, 5, 4, 0).unwrap(),
            Some(Action::EscalateManualReview)
        );
    }
}
