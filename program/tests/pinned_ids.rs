//! Base58 and Anchor-discriminator verification for every pinned external ID.
//!
//! The program hard-codes venue program IDs as raw `[u8; 32]` and Anchor
//! discriminators as raw `[u8; 8]`. A unit test that asserts a constant equals
//! the same literal proves only that the file parses — it re-states the value
//! instead of deriving it, so a wrong byte passes just as happily as a right
//! one, and the human-readable base58 in the doc comment is never checked at
//! all. (That is exactly how three different typo'd spellings of the Jupiter
//! address accumulated in `jupiter.rs` while the bytes stayed correct.)
//!
//! These tests derive instead:
//!
//!   * program IDs are base58-**encoded** from the pinned bytes and compared to
//!     the published address string, so the doc comment and the constant cannot
//!     drift apart, and
//!   * discriminators are re-**hashed** from their Anchor preimage.
//!
//! Lives in `tests/` because the library is `#![no_std]` and both `sha2` and
//! `bs58` want std here.

use sha2::{Digest, Sha256};

use wick_guard::drift::{DRIFT_PROGRAM_ID, PLACE_PERP_ORDER_DISCRIMINATOR};
use wick_guard::jupiter::{INSTANT_CREATE_TPSL_DISCRIMINATOR, JUPITER_PROGRAM_ID};
use wick_guard::pyth::{PRICE_UPDATE_V2_DISCRIMINATOR, PYTH_RECEIVER_PROGRAM_ID};

/// First 8 bytes of `sha256(preimage)` — how Anchor derives every discriminator.
fn anchor_discriminator(preimage: &str) -> [u8; 8] {
    let digest = Sha256::digest(preimage.as_bytes());
    let mut out = [0u8; 8];
    out.copy_from_slice(&digest[..8]);
    out
}

#[test]
fn drift_program_id_matches_published_base58() {
    // Velocity's `programs/drift/src/lib.rs` `declare_id!` — the live successor
    // to the decommissioned `dRifty...` program.
    assert_eq!(
        bs58::encode(DRIFT_PROGRAM_ID.to_bytes()).into_string(),
        "vELoC1audYbSYVRXn1vPaV8Axoa9oU6BYmNGZZBDZ1P",
    );
}

#[test]
fn jupiter_program_id_matches_published_base58() {
    assert_eq!(
        bs58::encode(JUPITER_PROGRAM_ID.to_bytes()).into_string(),
        "PERPHjGBqRHArX4DySjwM6UJHiR3sWAatqfdBS2qQJu",
    );
}

#[test]
fn pyth_receiver_program_id_matches_published_base58() {
    // `@pythnetwork/pyth-solana-receiver`'s `DEFAULT_RECEIVER_PROGRAM_ID`; the
    // same address on devnet and mainnet.
    assert_eq!(
        bs58::encode(PYTH_RECEIVER_PROGRAM_ID.to_bytes()).into_string(),
        "rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ",
    );
}

#[test]
fn place_perp_order_discriminator_is_derived() {
    assert_eq!(
        PLACE_PERP_ORDER_DISCRIMINATOR,
        anchor_discriminator("global:place_perp_order"),
    );
}

#[test]
fn instant_create_tpsl_discriminator_is_derived() {
    // Anchor snake_cases the handler name to build the preimage, so the IDL's
    // camelCase `instantCreateTpsl` hashes as `instant_create_tpsl`. Getting
    // this wrong yields a well-formed instruction the venue rejects.
    assert_eq!(
        INSTANT_CREATE_TPSL_DISCRIMINATOR,
        anchor_discriminator("global:instant_create_tpsl"),
    );
}

#[test]
fn price_update_v2_discriminator_is_derived() {
    // Account discriminators use the `account:` domain, not `global:`.
    assert_eq!(
        PRICE_UPDATE_V2_DISCRIMINATOR,
        anchor_discriminator("account:PriceUpdateV2"),
    );
}

/// Two venues that share a program ID would let a Drift-tagged guard dispatch
/// into Jupiter (or the reverse) if a copy-paste ever collapsed the constants.
#[test]
fn pinned_program_ids_are_distinct() {
    let ids = [
        ("drift", DRIFT_PROGRAM_ID.to_bytes()),
        ("jupiter", JUPITER_PROGRAM_ID.to_bytes()),
        ("pyth", PYTH_RECEIVER_PROGRAM_ID.to_bytes()),
    ];
    for (i, (a_name, a)) in ids.iter().enumerate() {
        for (b_name, b) in &ids[i + 1..] {
            assert_ne!(a, b, "{a_name} and {b_name} pin the same program ID");
        }
    }
}
