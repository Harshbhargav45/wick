//! Property tests for the §8.3 bounded partial-close solver.
//!
//! `solve_partial_close_fraction` is a 20-iteration binary search over
//! `f_bps ∈ [0, BPS_DENOM]`. Unit tests pin a handful of points; these tests
//! assert the two properties that actually matter across the whole input
//! space:
//!
//!   * **Safe** — closing the returned fraction really does restore the
//!     buffer. A solver that returns too small an `f` leaves the position
//!     under-collateralized after the guard has already committed its nonce.
//!   * **Minimal** — `f - 1` does not. A solver that returns too large an `f`
//!     closes more of the user's position than the policy required, which is
//!     an unrecoverable loss of exposure rather than a recoverable error.
//!
//! The predicate is re-derived here from the spec rather than imported, so a
//! sign error inside the solver's own `is_safe` closure cannot make these
//! tests agree with the bug.

// The predicate's arguments are exactly the solver's inputs. Grouping them into
// a struct would put a layer between the two, which is the one thing this file
// exists to avoid.
#![allow(clippy::too_many_arguments)]

use proptest::prelude::*;
use wick_guard::state::{solve_partial_close_fraction, BPS_DENOM, SCALE};

/// Is the position safe after closing `f_bps`, per §8.3?
///
/// Closing fraction `f` realizes `pnl*f` and keeps `pnl*(1-f)` unrealized, so
/// equity after the close is `collateral - fee + pnl` regardless of `f`; the
/// remaining `(1-f)` of the position still has to be covered by the buffer.
fn is_safe(
    collateral: u128,
    size: i128,
    entry: u128,
    current: u128,
    margin_bps: u128,
    buffer_bps: u128,
    fee_bps: u128,
    f_bps: u128,
) -> bool {
    let pnl = (size * (current as i128 - entry as i128)) / SCALE as i128;
    let notional = (size.unsigned_abs() * current) / SCALE;
    let m_full = (notional * margin_bps) / BPS_DENOM;
    let target_full = (m_full * (BPS_DENOM + buffer_bps)) / BPS_DENOM;

    let fee_full = (notional * fee_bps) / BPS_DENOM;
    let fee = (fee_full * f_bps) / BPS_DENOM;

    let equity_after = collateral as i128 - fee as i128 + pnl;
    let remaining_target = (target_full * (BPS_DENOM - f_bps)) / BPS_DENOM;

    equity_after >= remaining_target as i128
}

prop_compose! {
    /// Inputs bounded well inside u128 so the reference predicate above can use
    /// plain arithmetic — this is about solver correctness, not overflow
    /// handling, which `checked_*` covers in the solver itself.
    fn position()(
        // $1 .. $10M collateral, 6dp fixed point.
        collateral in 1_000_000u128..10_000_000_000_000u128,
        // 0.001 .. 1000 units, long or short.
        size_abs in 1_000u128..1_000_000_000u128,
        is_long in any::<bool>(),
        // $0.01 .. $100k per unit.
        entry in 10_000u128..100_000_000_000u128,
        current in 10_000u128..100_000_000_000u128,
        // 0.5% .. 20% maintenance margin.
        margin_bps in 50u128..2_000u128,
        // 0% .. 50% trigger buffer over maintenance.
        buffer_bps in 0u128..5_000u128,
        // 0 .. 1% fee.
        fee_bps in 0u128..100u128,
    ) -> (u128, i128, u128, u128, u128, u128, u128) {
        let size = if is_long { size_abs as i128 } else { -(size_abs as i128) };
        (collateral, size, entry, current, margin_bps, buffer_bps, fee_bps)
    }
}

proptest! {
    /// The returned fraction is always in range and always restores the buffer.
    #[test]
    fn solution_is_in_range_and_safe(p in position()) {
        let (collateral, size, entry, current, margin_bps, buffer_bps, fee_bps) = p;

        let Ok(f) = solve_partial_close_fraction(
            collateral, size, entry, current, margin_bps, buffer_bps, fee_bps,
        ) else {
            // `CannotReachSafeBuffer` is a legitimate answer for an insolvent
            // position; `solver_rejects_only_when_full_close_fails` pins when.
            return Ok(());
        };

        prop_assert!(f <= BPS_DENOM, "fraction {f} out of range");
        prop_assert!(
            is_safe(collateral, size, entry, current, margin_bps, buffer_bps, fee_bps, f),
            "returned f={f} does not restore the buffer",
        );
    }

    /// Nothing smaller works — the guard never closes more than it has to.
    #[test]
    fn solution_is_minimal(p in position()) {
        let (collateral, size, entry, current, margin_bps, buffer_bps, fee_bps) = p;

        let Ok(f) = solve_partial_close_fraction(
            collateral, size, entry, current, margin_bps, buffer_bps, fee_bps,
        ) else {
            return Ok(());
        };

        if f == 0 {
            return Ok(()); // Already safe; there is nothing below it.
        }

        prop_assert!(
            !is_safe(collateral, size, entry, current, margin_bps, buffer_bps, fee_bps, f - 1),
            "f={f} is not minimal — closing {} also restores the buffer",
            f - 1,
        );
    }

    /// A position that is already above its buffer needs no close at all.
    #[test]
    fn already_safe_positions_solve_to_zero(p in position()) {
        let (collateral, size, entry, current, margin_bps, buffer_bps, fee_bps) = p;

        prop_assume!(is_safe(
            collateral, size, entry, current, margin_bps, buffer_bps, fee_bps, 0,
        ));

        let f = solve_partial_close_fraction(
            collateral, size, entry, current, margin_bps, buffer_bps, fee_bps,
        ).expect("a position that is already safe must solve");

        prop_assert_eq!(f, 0, "already-safe position asked to close {}", f);
    }

    /// The solver only refuses when a full close cannot save the position —
    /// i.e. equity is non-positive even after realizing everything. Refusing
    /// while a solution exists would strand a position the guard could have
    /// rescued.
    #[test]
    fn solver_rejects_only_when_full_close_fails(p in position()) {
        let (collateral, size, entry, current, margin_bps, buffer_bps, fee_bps) = p;

        let result = solve_partial_close_fraction(
            collateral, size, entry, current, margin_bps, buffer_bps, fee_bps,
        );

        if result.is_err() {
            prop_assert!(
                !is_safe(collateral, size, entry, current, margin_bps, buffer_bps, fee_bps, BPS_DENOM),
                "solver refused a position that a full close would have saved",
            );
        }
    }
}
