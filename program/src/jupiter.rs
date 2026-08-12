//! Jupiter Perps venue adapter — co-signed safety-net boundary (§8.7).
//!
//! Drift is the autonomous tier: Wick's guard PDA *is* the position delegate
//! and signs CPIs itself. Jupiter is the **co-signed** tier: every state change
//! requires the position **owner**'s signature, and Jupiter's keeper-gated
//! flows additionally require `keeper`/`apiKeeper` signer infrastructure that a
//! guard neither has nor should fake. Wick therefore:
//!
//! * never executes Jupiter instructions autonomously (`execute_autonomous` in
//!   `processor.rs` rejects the venue — no fake autonomy), and
//! * builds the deterministic **safety-net** instruction data
//!   (`instant_create_tpsl`, the TP/SL floor) that the owner — and only the
//!   owner — signs and lands (§8.4 CoSigned, §3 Phase-3 safety-net cut).
//!
//! This module owns the serialization that the client mirrors to reconstruct
//! the owner-signed transaction, so it is the single source of truth for the
//! Jupiter instruction shape — the same role `drift.rs` plays for the
//! autonomous tier, but build-only.
//!
//! Jupiter's safety-net instruction `instantCreateTpsl` (Anchor borsh):
//!   program id `PERPHjGBqRHArX4DySjwM6UJHiR3sWAatqfdBS2qQJu`
//!   data       `[ global:instant_create_tpsl discriminator (8) ]`
//!              `[ InstantCreateTpslParams (42) ]`

use pinocchio::Address;

use crate::error::WickError;

/// `state.venue` tag for the Jupiter adapter.
pub const VENUE_JUPITER: u8 = 2;

/// Jupiter Perpetuals program ID (mainnet-beta).
///
/// `PERPHjGBqRHArX4DySjwM6UJHiR3sWAatqfdBS2qQJu` — verified against the
/// jupiter-perps IDL and the on-chain program. The base58 above is not a
/// comment on trust: `tests/pinned_ids.rs::jupiter_program_id_matches_published_base58`
/// encodes these bytes and asserts they spell it, so the two cannot drift apart.
pub const JUPITER_PROGRAM_ID: Address = Address::new_from_array([
    5, 177, 243, 202, 241, 148, 98, 239, 135, 96, 240, 171, 222, 117, 205, 61, 158, 227, 27, 58,
    50, 198, 32, 232, 148, 18, 46, 156, 155, 129, 69, 250,
]);

/// Anchor discriminator of `instantCreateTpsl` = the first 8 bytes of
/// `sha256("global:instant_create_tpsl")` — computed and pinned, not guessed.
///
/// Anchor snake_cases the Rust handler name to build the preimage, so the
/// camelCase IDL entry `instantCreateTpsl` hashes as `instant_create_tpsl`.
pub const INSTANT_CREATE_TPSL_DISCRIMINATOR: [u8; 8] = [117, 98, 66, 127, 30, 50, 73, 185];

/// `InstantCreateTpslParams` (Anchor borsh) that define the safety-net TP/SL
/// the owner signs into place. Field order and widths come from the audited
/// Jupiter IDL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TpslParams {
    /// Collateral delta in USD (6dp).
    pub collateral_usd_delta: u64,
    /// Position size delta in USD (6dp).
    pub size_usd_delta: u64,
    /// Trigger price (6dp).
    pub trigger_price: u64,
    /// When `true` the order triggers when price rises above `trigger_price`.
    pub trigger_above_threshold: bool,
    /// When `true` the order applies to the entire position.
    pub entire_position: bool,
    /// Bump used by the keeper to dedupe identical requests.
    pub counter: u64,
    /// Client request timestamp (unix seconds).
    pub request_time: i64,
}

impl TpslParams {
    pub const LEN: usize = 42;

    /// Serialize borsh: `u64,u64,u64,bool,bool,u64,i64` (all little-endian).
    pub fn try_to_data(&self) -> Result<[u8; Self::LEN], WickError> {
        let mut d = [0u8; Self::LEN];
        d[0..8].copy_from_slice(&self.collateral_usd_delta.to_le_bytes());
        d[8..16].copy_from_slice(&self.size_usd_delta.to_le_bytes());
        d[16..24].copy_from_slice(&self.trigger_price.to_le_bytes());
        d[24] = self.trigger_above_threshold as u8;
        d[25] = self.entire_position as u8;
        d[26..34].copy_from_slice(&self.counter.to_le_bytes());
        d[34..42].copy_from_slice(&self.request_time.to_le_bytes());
        Ok(d)
    }
}

/// Build the full `instantCreateTpsl` instruction data (discriminator + params)
/// that the owner signs. Build-only: this module never invokes the instruction.
pub fn build_instant_tpsl_data(
    params: &TpslParams,
) -> Result<[u8; 8 + TpslParams::LEN], WickError> {
    let mut d = [0u8; 8 + TpslParams::LEN];
    d[..8].copy_from_slice(&INSTANT_CREATE_TPSL_DISCRIMINATOR);
    d[8..].copy_from_slice(&params.try_to_data()?);
    Ok(d)
}

/// Map a take-profit price (6dp) to the full Jupiter safety-net instruction
/// data: a take-order above the current market, covering the entire position.
/// Callers decide the expected nonce to stamp beside it.
pub fn build_tp_safety_net(take_profit: u128, request_time: i64) -> Result<[u8; 50], WickError> {
    let price = u64::try_from(take_profit).map_err(|_| WickError::MathOverflow)?;
    let params = TpslParams {
        collateral_usd_delta: 0,
        size_usd_delta: 0,
        trigger_price: price,
        trigger_above_threshold: true,
        entire_position: true,
        counter: 0,
        request_time,
    };
    build_instant_tpsl_data(&params)
}

/// Build the **defensive** leg of the safety net: a partial stop that closes
/// `size_usd` of notional at `trigger_price`.
///
/// `build_tp_safety_net` covers the pleasant half of the problem — a guard that
/// only ever hands its owner a take-profit is silent during the breach it
/// exists to answer. On a co-signed venue the guard cannot place the reducing
/// order itself (§8.7), so the most it can honestly do is build the exact
/// instruction that answers the breach and let the owner sign it. This is that
/// instruction.
///
/// Two things distinguish it from the take-profit leg:
///
/// * **Direction.** A stop protecting a long sits *below* market; protecting a
///   short it sits *above*. Getting this backwards would build an order that
///   fires immediately on the wrong side, so it is derived from the position's
///   sign rather than passed as a bare flag by the caller.
/// * **A TTL.** A stop is a price level, and a level is only meaningful against
///   the mark it was derived from. If the price backing this build is older than
///   `ttl_secs` the build is refused with `DefensiveCloseUnavailable` rather than
///   handing the owner a level to sign that the market may already be through.
///   The same check rejects a future-dated price, which means clock skew between
///   the oracle and the validator — a state where "how stale is this" has no
///   answer worth trusting.
///
/// `entire_position` is false: the solver sized this close deliberately (§8.2),
/// and rounding it up to the whole position would exit a trade the policy only
/// asked to trim.
pub fn build_defensive_close(
    trigger_price: u128,
    size_usd: u128,
    position_is_short: bool,
    price_publish_ts: i64,
    now: i64,
    ttl_secs: i64,
) -> Result<[u8; 50], WickError> {
    let age = now
        .checked_sub(price_publish_ts)
        .ok_or(WickError::MathOverflow)?;
    if !(0..=ttl_secs).contains(&age) {
        return Err(WickError::DefensiveCloseUnavailable);
    }
    // A stop that closes nothing is not a stop. Refusing here keeps the guard
    // from parking a no-op instruction in `pending_ix`, which would read on the
    // console as "the owner has something to sign" when they do not.
    if size_usd == 0 || trigger_price == 0 {
        return Err(WickError::DefensiveCloseUnavailable);
    }
    let params = TpslParams {
        collateral_usd_delta: 0,
        size_usd_delta: u64::try_from(size_usd).map_err(|_| WickError::MathOverflow)?,
        trigger_price: u64::try_from(trigger_price).map_err(|_| WickError::MathOverflow)?,
        trigger_above_threshold: position_is_short,
        entire_position: false,
        counter: 0,
        request_time: now,
    };
    build_instant_tpsl_data(&params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tp_safety_net_is_full_above_market() {
        let d = build_tp_safety_net(60_000_000, 1_700_000_000).unwrap();
        assert_eq!(&d[..8], &INSTANT_CREATE_TPSL_DISCRIMINATOR);
        let p = TpslParams {
            collateral_usd_delta: 0,
            size_usd_delta: 0,
            trigger_price: 60_000_000,
            trigger_above_threshold: true,
            entire_position: true,
            counter: 0,
            request_time: 1_700_000_000,
        };
        assert_eq!(&d[8..], &p.try_to_data().unwrap());
    }

    #[test]
    fn tpsl_params_borsh_layout() {
        let p = TpslParams {
            collateral_usd_delta: 1,
            size_usd_delta: 2,
            trigger_price: 3,
            trigger_above_threshold: true,
            entire_position: false,
            counter: 4,
            request_time: -5,
        };
        let d = p.try_to_data().unwrap();
        assert_eq!(d[0..8], 1u64.to_le_bytes());
        assert_eq!(d[8..16], 2u64.to_le_bytes());
        assert_eq!(d[16..24], 3u64.to_le_bytes());
        assert_eq!(d[24], 1); // trigger_above_threshold
        assert_eq!(d[25], 0); // entire_position
        assert_eq!(d[26..34], 4u64.to_le_bytes());
        assert_eq!(d[34..42], (-5i64).to_le_bytes());
    }

    #[test]
    fn build_data_prefixes_discriminator() {
        let p = TpslParams {
            collateral_usd_delta: 0,
            size_usd_delta: 0,
            trigger_price: 100_000_000,
            trigger_above_threshold: true,
            entire_position: true,
            counter: 0,
            request_time: 0,
        };
        let d = build_instant_tpsl_data(&p).unwrap();
        assert_eq!(&d[..8], &INSTANT_CREATE_TPSL_DISCRIMINATOR);
        assert_eq!(d.len(), 8 + TpslParams::LEN);
    }

    // ------------------------------------------------------------------
    //  Defensive close (§8.7) — the leg that answers a live breach.
    // ------------------------------------------------------------------

    /// The direction is the whole safety argument: a stop for a **long** must
    /// trigger on the way *down*, so `trigger_above_threshold` must be false.
    /// Inverted, the order fires the instant it lands and closes a healthy leg.
    #[test]
    fn defensive_close_for_a_long_triggers_below_market() {
        let d = build_defensive_close(
            48_000_000,
            25_000_000,
            false,
            1_700_000_000,
            1_700_000_010,
            60,
        )
        .unwrap();
        assert_eq!(&d[..8], &INSTANT_CREATE_TPSL_DISCRIMINATOR);
        assert_eq!(d[8 + 24], 0); // trigger_above_threshold = false
        assert_eq!(d[8 + 25], 0); // entire_position = false — the solver sized it
        assert_eq!(d[8 + 8..8 + 16], 25_000_000u64.to_le_bytes()); // size_usd_delta
        assert_eq!(d[8 + 16..8 + 24], 48_000_000u64.to_le_bytes()); // trigger_price
        assert_eq!(d[8..8 + 8], 0u64.to_le_bytes()); // no collateral delta
        assert_eq!(d[8 + 34..8 + 42], 1_700_000_010i64.to_le_bytes()); // request_time = now
    }

    /// The mirror image: a short is protected by a stop *above* market.
    #[test]
    fn defensive_close_for_a_short_triggers_above_market() {
        let d = build_defensive_close(
            52_000_000,
            25_000_000,
            true,
            1_700_000_000,
            1_700_000_000,
            60,
        )
        .unwrap();
        assert_eq!(d[8 + 24], 1);
    }

    /// A level is only meaningful against the mark it came from. Past the TTL
    /// the build is refused rather than handing the owner a stale price to sign.
    #[test]
    fn defensive_close_refuses_a_stale_price() {
        assert_eq!(
            build_defensive_close(48_000_000, 1, false, 1_700_000_000, 1_700_000_061, 60)
                .unwrap_err(),
            WickError::DefensiveCloseUnavailable
        );
        // Exactly at the TTL is still inside it.
        assert!(
            build_defensive_close(48_000_000, 1, false, 1_700_000_000, 1_700_000_060, 60).is_ok()
        );
    }

    /// A future-dated price means oracle/validator clock skew, a state where
    /// "how stale is this" has no trustworthy answer. Refuse rather than guess.
    #[test]
    fn defensive_close_refuses_a_future_dated_price() {
        assert_eq!(
            build_defensive_close(48_000_000, 1, false, 1_700_000_001, 1_700_000_000, 60)
                .unwrap_err(),
            WickError::DefensiveCloseUnavailable
        );
    }

    /// A stop that closes nothing, or triggers at zero, is not a stop — parking
    /// one in `pending_ix` would read on the console as a real thing to sign.
    #[test]
    fn defensive_close_refuses_degenerate_orders() {
        assert_eq!(
            build_defensive_close(48_000_000, 0, false, 1_700_000_000, 1_700_000_000, 60)
                .unwrap_err(),
            WickError::DefensiveCloseUnavailable
        );
        assert_eq!(
            build_defensive_close(0, 25_000_000, false, 1_700_000_000, 1_700_000_000, 60)
                .unwrap_err(),
            WickError::DefensiveCloseUnavailable
        );
    }

    /// Both legs are `instant_create_tpsl`, so the bytes alone cannot tell a
    /// take-profit from a stop. That is exactly why `PendingIx.kind` exists —
    /// this test pins the two builders as genuinely distinct payloads so a
    /// console cannot render one as the other.
    #[test]
    fn defensive_close_and_take_profit_are_distinguishable() {
        let tp = build_tp_safety_net(60_000_000, 1_700_000_000).unwrap();
        let close = build_defensive_close(
            48_000_000,
            25_000_000,
            false,
            1_700_000_000,
            1_700_000_000,
            60,
        )
        .unwrap();
        assert_ne!(tp, close);
        assert_eq!(tp[8 + 25], 1); // TP closes the entire position...
        assert_eq!(close[8 + 25], 0); // ...the defensive leg closes a slice.
    }

    /// Notional above `u64::MAX` cannot be expressed in the venue's own field
    /// widths; overflowing into a wrapped size would place an order for the
    /// wrong amount.
    #[test]
    fn defensive_close_rejects_unrepresentable_size() {
        assert_eq!(
            build_defensive_close(
                48_000_000,
                u128::from(u64::MAX) + 1,
                false,
                1_700_000_000,
                1_700_000_000,
                60
            )
            .unwrap_err(),
            WickError::MathOverflow
        );
    }
}
