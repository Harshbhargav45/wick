//! Mock Drift program for litesvm integration tests.
//!
//! Real Drift's `place_perp_order` enforces a large account graph (perp
//! market, oracle, spot markets, user maps) that a test fixture cannot
//! reproduce without the full protocol deploy. This mock models the one
//! invariant Wick's autonomous tier actually depends on: the order's
//! **authority** (account index 2) must be a signer AND must match the
//! `delegate` stored in the user account (account index 1). It mirrors Drift's
//! `can_sign_for_user` (`user.authority == signer || user.delegate == signer`)
//! plus the hard `reduce_only` flag, and writes a `REDUCED_MARKER` into the
//! user account so a test can prove the guard-PDA-signed reduce CPI landed end
//! to end.

#![no_std]

use pinocchio::{error::ProgramError, program_entrypoint, AccountView, ProgramResult};

/// Anchor discriminator of `global:place_perp_order` — must match `drift.rs`.
pub const PLACE_PERP_ORDER_DISCRIMINATOR: [u8; 8] = [69, 161, 93, 202, 120, 126, 76, 185];

/// Written into `user` data on a successful reduce — `b"REDUCE"`.
pub const REDUCED_MARKER: [u8; 6] = *b"REDUCE";

/// Byte offset of `User.delegate` in Drift's zero-copy `User` layout.
/// Anchor prefix (8) + `authority: Pubkey` (32) => delegate at 8 + 32 = 40.
/// Verified against `state/user.rs`: `User { authority, delegate, ... }`.
const DELEGATE_OFFSET: usize = 40;

/// Byte offset of `reduce_only` within the 42-byte `place_perp_order` data
/// (8-byte discriminator + 34-byte borsh `OrderParams`; `reduce_only: bool`
/// sits at byte 30, unchanged from Drift's layout — Velocity only trails two
/// extra `Option` fields). Must match `drift.rs::ReduceOrderParams::try_to_data`.
const REDUCE_ONLY_OFFSET: usize = 30;

program_entrypoint!(process_instruction);

#[cfg(not(feature = "no-entrypoint"))]
pinocchio::default_allocator!();

#[cfg(all(not(feature = "no-entrypoint"), not(test)))]
pinocchio::nostd_panic_handler!();

pub fn process_instruction(
    _program_id: &pinocchio::Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if data.len() < 8 || data[..8] != PLACE_PERP_ORDER_DISCRIMINATOR {
        return Err(ProgramError::InvalidInstructionData);
    }

    // Index 1 is `user` (AccountLoader<User>). Its `delegate` is the only
    // actor allowed to place orders on the authority's behalf.
    let user = accounts.get(1).ok_or(ProgramError::NotEnoughAccountKeys)?;
    let user_data = user
        .try_borrow()
        .map_err(|_| ProgramError::AccountBorrowFailed)?;
    let mut delegate = [0u8; 32];
    let n = user_data
        .get(DELEGATE_OFFSET..DELEGATE_OFFSET + 32)
        .ok_or(ProgramError::AccountDataTooSmall)?
        .len();
    delegate[..n].copy_from_slice(&user_data[DELEGATE_OFFSET..DELEGATE_OFFSET + n]);
    drop(user_data);

    // Index 2 is `authority` (Signer). Drift's `can_sign_for_user` accepts the
    // signer iff it is the user's delegate (or the user's own authority). The
    // autonomous tier signs as the guard PDA, which the operator registered as
    // the delegate — so the delegate-derived PDA must sign, not just any
    // signer.
    let authority = accounts.get(2).ok_or(ProgramError::NotEnoughAccountKeys)?;
    if !authority.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if authority.address().as_ref() != delegate.as_slice() {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Wick hard-bakes `reduce_only = true`. Reject any order that would add
    // exposure — the mock enforces the adapter's claim on its side too.
    if data.len() <= REDUCE_ONLY_OFFSET || data[REDUCE_ONLY_OFFSET] != 1 {
        return Err(ProgramError::InvalidInstructionData);
    }

    // Mark the user account as reduced — proves the CPI landed end to end.
    let mut user_data = user
        .try_borrow_mut()
        .map_err(|_| ProgramError::AccountBorrowFailed)?;
    let n = core::cmp::min(user_data.len(), REDUCED_MARKER.len());
    user_data[..n].copy_from_slice(&REDUCED_MARKER[..n]);
    Ok(())
}
