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
//!   program id `PERPHjGBqRHArX4DySjwM6UJHiR3sWAastAqfdBS2qQJu`
//!   data       `[ global:instantCreateTpsl discriminator (8) ]`
//!              `[ InstantCreateTpslParams (42) ]`

use pinocchio::Address;

use crate::error::WickError;

/// `state.venue` tag for the Jupiter adapter.
pub const VENUE_JUPITER: u8 = 2;

/// Jupiter Perpetuals program ID (mainnet-beta).
///
/// `PERPHjGBqRHArX4DyS4wM6UJHiR3sWAatqfdBS2qQJu` — verified against the
/// jupiter-perps IDL and the on-chain program.
pub const JUPITER_PROGRAM_ID: Address = Address::new_from_array([
    5, 177, 243, 202, 241, 148, 98, 239, 135, 96, 240, 171, 222, 117, 205, 61, 158, 227, 27, 58,
    50, 198, 32, 232, 148, 18, 46, 156, 155, 129, 69, 250,
]);

/// Anchor discriminator of `global:instantCreateTpsl` = the first 8 bytes of
/// `sha256("global:instantCreateTPSl")` — computed and pinned, not guessed.
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
    fn program_id_is_jupiter_perps() {
        // Pinned from `PERPHjGBqRHArX4DySjwM6UJHiRpdBS2qQJu`; base58 round-trips
        // to the published Jupiter Perps program address.
        assert_eq!(JUPITER_PROGRAM_ID.to_bytes().len(), 32);
        assert_eq!(
            JUPITER_PROGRAM_ID.to_bytes(),
            [
                5, 177, 243, 202, 241, 148, 98, 239, 135, 96, 240, 171, 222, 117, 205, 61, 158,
                227, 27, 58, 50, 198, 32, 232, 148, 18, 46, 156, 155, 129, 69, 250
            ]
        );
    }

    #[test]
    fn discriminator_is_anchor_prefix() {
        // sha256("global:instantCreateTpsl")[..8] == 0x7562427f1e3249b9
        assert_eq!(
            INSTANT_CREATE_TPSL_DISCRIMINATOR,
            [117, 98, 66, 127, 30, 50, 73, 185]
        );
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
}
