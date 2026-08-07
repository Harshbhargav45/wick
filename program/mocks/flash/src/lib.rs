//! Mock Flash perpetuals program for litesvm integration tests.
//!
//! Real Flash's `close_position` enforces 12 accounts and a 6-signature CPI
//! dance that a test fixture cannot reproduce without the full Flash deploy.
//! This mock models the one invariant Wick's autonomous tier actually depends
//! on: the **owner** of the position must be a signer of the CPI. It mirrors
//! Flash's account order (`owner` at index 0, `position` at index 5) and
//! writes a `CLOSED_MARKER` into the position account so a test can prove the
//! guard-PDA-signed `close_position` CPI landed end to end.

#![no_std]

use pinocchio::{error::ProgramError, program_entrypoint, AccountView, ProgramResult};

/// Anchor discriminator of `global:close_position` — must match `flash.rs`.
pub const CLOSE_POSITION_DISCRIMINATOR: [u8; 8] = [123, 134, 81, 0, 49, 68, 98, 98];

/// Written into `position` data on a successful close — `b"WICK"`.
pub const CLOSED_MARKER: [u8; 4] = *b"WICK";

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
    if data.len() < 8 || data[..8] != CLOSE_POSITION_DISCRIMINATOR {
        return Err(ProgramError::InvalidInstructionData);
    }

    // Index 0 is `owner` (Wick's guard PDA in the autonomous tier).
    let owner = accounts.first().ok_or(ProgramError::NotEnoughAccountKeys)?;
    if !owner.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Index 5 is `position`. Mark it closed.
    let position = accounts.get(5).ok_or(ProgramError::NotEnoughAccountKeys)?;
    let mut position_data = position
        .try_borrow_mut()
        .map_err(|_| ProgramError::AccountBorrowFailed)?;
    let n = core::cmp::min(position_data.len(), CLOSED_MARKER.len());
    position_data[..n].copy_from_slice(&CLOSED_MARKER[..n]);
    Ok(())
}
