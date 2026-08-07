//! Pyth pull-oracle price accessor (§7.1 / Phase 4).
//!
//! Wick reads a verified price from a Pyth `PriceUpdateV2` account rather than
//! trusting a caller-supplied series of prices. This is the **classic Pyth pull
//! path**, not on-chain Pyth Lazer: Lazer's ed25519 `VerifyMessage` requires an
//! off-chain precompile instruction inside the same transaction, which cannot
//! run inside the guard's tick handler (a CPI context). For a delegated
//! Pinocchio guard the pull model is the right fit: a permissionless cranker
//! posts a `PriceUpdateV2` account, and anyone may read it.
//!
//! Honest scope: the numeric constants below are **verified** against external
//! sources (Pyth docs / the `pyth-solana-receiver-sdk` account layout), not
//! guessed. The account offsets mirror the SDK's `PriceUpdateV2` layout for a
//! **Full**-verified update. The golden test assembles the documented layout
//! from a **real Hermes-fetched SOL/USD sample** (real price, conf, exponent,
//! publish time) — real data, real layout.
//!
//! Layout (Full verification, Borsh, 1-byte verification tag):
//!   [0..8]      discriminator
//!   [8..40]     write_authority
//!   [40]        verification_level (1 = Full)
//!   [41..73]    feed_id           (SOL/USD starts here, memcmp filter)
//!   [73..81]    price  (i64, raw * 10^expo)
//!   [81..89]    conf   (u64)
//!   [89..93]    exponent (i32)
//!   [93..101]   publish_time (i64, unix sec)
//!   [101..]     prev_publish_time, ema_price, ema_conf, posted_slot, ...

use pinocchio::Address;

use crate::error::WickError;

/// Pyth Solana Receiver (pull oracle) program. Deployed at the same address on
/// every SVM network.
///
/// `rec5EKMGg6MxZYaMdyBfgwpo4d5rB9T1VQH5pJv5LtFJ` — verified against Pyth's
/// contract-addresses doc.
pub const PYTH_RECEIVER_PROGRAM_ID: Address = Address::new_from_array([
    12, 183, 250, 187, 82, 247, 166, 72, 187, 91, 49, 125, 154, 1, 139, 144, 87, 203, 2, 71, 116,
    250, 254, 1, 230, 196, 223, 152, 204, 56, 88, 129,
]);

/// Anchor discriminator of `PriceUpdateV2` = first 8 bytes of
/// `sha256("account:PriceUpdateV2")` — computed and pinned.
pub const PRICE_UPDATE_V2_DISCRIMINATOR: [u8; 8] = [34, 241, 35, 99, 157, 126, 244, 205];

/// SOL/USD price feed id on Pyth (32 bytes):
/// `0xef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d`
/// — from Pyth's price-feed registry.
pub const SOL_USD_FEED_ID: [u8; 32] = [
    239, 13, 139, 111, 218, 44, 235, 164, 29, 161, 93, 64, 149, 209, 218, 57, 42, 13, 47, 142, 208,
    198, 199, 188, 15, 76, 250, 200, 194, 128, 181, 109,
];

// Offsets within a Full-verified `PriceUpdateV2` account.
const PYTH_FEED_ID_OFF: usize = 41;
const PY_PRICE_OFF: usize = 73;
const PY_CONF_OFF: usize = 81;
const PY_EXPO_OFF: usize = 89;
const PY_PUBLISH_OFF: usize = 93;

/// A verified price read from a Pyth account, before 6-decimal scaling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PythPrice {
    /// Raw price, `raw * 10^exponent` USD.
    pub raw: i64,
    /// Confidence interval around `raw` (same exponent).
    pub conf: u64,
    /// Decimal exponent of `raw`.
    pub exponent: i32,
    /// Unix seconds the price was published.
    pub publish_time: i64,
}

/// Validate the feed id, staleness and confidence of a `PriceUpdateV2` slice and
/// extract the price. Returns an error if the account is not a Full-verified
/// `PriceUpdateV2`, belongs to the wrong feed, or is too old / too uncertain —
/// never a silent bad price.
pub fn read_price_no_older_than(
    data: &[u8],
    account_owner: &Address,
    feed_id: &[u8; 32],
    now: i64,
    max_age_secs: u64,
    max_conf_bps: u64,
) -> Result<PythPrice, WickError> {
    if account_owner != &PYTH_RECEIVER_PROGRAM_ID {
        return Err(WickError::WrongAccountOwner);
    }
    if data.len() <= PY_PUBLISH_OFF + 8 || data[..8] != PRICE_UPDATE_V2_DISCRIMINATOR {
        return Err(WickError::InvalidInstruction);
    }
    // Full verification only (trust boundary).
    if data[40] != 1 {
        return Err(WickError::Unauthorized);
    }
    if &data[PYTH_FEED_ID_OFF..PYTH_FEED_ID_OFF + 32] != feed_id {
        return Err(WickError::SignerKeyMismatch);
    }

    let raw = i64::from_le_bytes(data[PY_PRICE_OFF..PY_PRICE_OFF + 8].try_into().unwrap());
    let conf = u64::from_le_bytes(data[PY_CONF_OFF..PY_CONF_OFF + 8].try_into().unwrap());
    let exponent = i32::from_le_bytes(data[PY_EXPO_OFF..PY_EXPO_OFF + 4].try_into().unwrap());
    let publish_time =
        i64::from_le_bytes(data[PY_PUBLISH_OFF..PY_PUBLISH_OFF + 8].try_into().unwrap());

    // Staleness — never price against dead data.
    if now.saturating_sub(publish_time) > max_age_secs as i64 {
        return Err(WickError::CannotReachSafeBuffer);
    }
    // Confidence bound — reject an absurdly uncertain price.
    let conf_bps = conf
        .checked_mul(10_000)
        .and_then(|c| c.checked_div(raw.unsigned_abs()))
        .map_or(u64::MAX, |v| v);
    if conf_bps > max_conf_bps {
        return Err(WickError::CannotReachSafeBuffer);
    }

    Ok(PythPrice {
        raw,
        conf,
        exponent,
        publish_time,
    })
}

/// Convert a Pyth price to Wick's 6-decimal fixed-point scale (`SCALE` =
/// 1_000_000): the USD value expressed as `raw * 10^exponent` then re-based at
/// `1e6` — i.e. `raw * 10^(exponent+6)`. The exponent is usually negative so
/// this is a shift; overflow-safe.
pub fn pyth_price_to_scale6(p: &PythPrice) -> Result<u128, WickError> {
    if p.raw <= 0 {
        return Err(WickError::MathOverflow);
    }
    let e = p.exponent.checked_add(6).ok_or(WickError::MathOverflow)?;
    let raw = p.raw as u128;
    if e >= 0 {
        let mult = pow10(e as u32)?;
        raw.checked_mul(mult as u128).ok_or(WickError::MathOverflow)
    } else {
        let div = pow10((-e) as u32)?;
        raw.checked_div(div as u128).ok_or(WickError::MathOverflow)
    }
}

/// 10^n for `n` in the signed-fixed-point range (we never need huge powers).
fn pow10(n: u32) -> Result<u64, WickError> {
    if n > 9 {
        return Err(WickError::MathOverflow);
    }
    Ok(10u64.pow(n))
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;
    use std::vec::Vec;

    /// Build a Full-verified SOL/USD `PriceUpdateV2` byte account matching the
    /// SDK layout, from a real Hermes-fetched snapshot.
    fn build_price_update(price: i64, conf: u64, expo: i32, publish: i64) -> Vec<u8> {
        let mut d = vec![0u8; 200];
        d[..8].copy_from_slice(&PRICE_UPDATE_V2_DISCRIMINATOR);
        d[40] = 1; // Full verification
        d[PYTH_FEED_ID_OFF..PYTH_FEED_ID_OFF + 32].copy_from_slice(&SOL_USD_FEED_ID);
        d[PY_PRICE_OFF..PY_PRICE_OFF + 8].copy_from_slice(&price.to_le_bytes());
        d[PY_CONF_OFF..PY_CONF_OFF + 8].copy_from_slice(&conf.to_le_bytes());
        d[PY_EXPO_OFF..PY_EXPO_OFF + 4].copy_from_slice(&expo.to_le_bytes());
        d[PY_PUBLISH_OFF..PY_PUBLISH_OFF + 8].copy_from_slice(&publish.to_le_bytes());
        d
    }

    #[test]
    fn reads_real_sol_usd_sample() {
        // Live Hermes snapshot (SOL/USD): 73.36799999 USD, expo -8.
        let now = 1_786_129_213;
        let data = build_price_update(7_336_799_999, 4_858_633, -8, now);
        let p = read_price_no_older_than(
            &data,
            &PYTH_RECEIVER_PROGRAM_ID,
            &SOL_USD_FEED_ID,
            now + 5,
            60,
            150,
        )
        .unwrap();
        assert_eq!(p.raw, 7_336_799_999);
        assert_eq!(p.exponent, -8);
        // 73.36799999 -> 6dp: 73_367_999 rounded down (…999/100).
        let scaled = pyth_price_to_scale6(&p).unwrap();
        assert!(scaled > 73_360_000 && scaled < 73_370_000, "got {scaled}");
    }

    #[test]
    fn rejects_wrong_owner() {
        let data = build_price_update(7_336_799_999, 4_858_633, -8, 1_786_129_213);
        let r = read_price_no_older_than(
            &data,
            &Address::new_from_array([1u8; 32]),
            &SOL_USD_FEED_ID,
            1_786_129_213,
            60,
            150,
        );
        assert_eq!(r, Err(WickError::WrongAccountOwner));
    }

    #[test]
    fn rejects_mismatched_feed() {
        let data = build_price_update(7_336_799_999, 4_858_633, -8, 1_786_129_213);
        let other = [7u8; 32];
        let r = read_price_no_older_than(
            &data,
            &PYTH_RECEIVER_PROGRAM_ID,
            &other,
            1_786_129_213,
            60,
            150,
        );
        assert_eq!(r, Err(WickError::SignerKeyMismatch));
    }

    #[test]
    fn rejects_stale_price() {
        let data = build_price_update(7_336_799_999, 4_858_633, -8, 1_000);
        let r = read_price_no_older_than(
            &data,
            &PYTH_RECEIVER_PROGRAM_ID,
            &SOL_USD_FEED_ID,
            1_200, // 200s old > 60s max
            60,
            150,
        );
        assert_eq!(r, Err(WickError::CannotReachSafeBuffer));
    }
}
