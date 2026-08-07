//! Ephemeral Rollup delegation hooks (§8.6).
//!
//! Wick's guard PDA can be delegated to the MagicBlock Ephemeral Rollup so the
//! FlashTrade venue can execute sub-50ms autonomous guard checks on delegated
//! state. These handlers wrap the Pinocchio-native
//! `ephemeral-rollups-pinocchio` SDK:
//!
//! * `delegate` — moves the guard PDA into the ER (called on base layer).
//! * `commit_and_undelegate` — schedules a state sync and returns ownership.
//! * `commit` — schedules a state sync, keeping the guard delegated.
//! * the undelegation callback — reverts ownership once the ER validator
//!   finishes the session (called by the Delegation Program via CPI).

use pinocchio::error::ProgramResult;
use pinocchio::AccountView;

use ephemeral_rollups_pinocchio::instruction::{delegate_account, undelegate};
use ephemeral_rollups_pinocchio::intent_bundle::MagicIntentBundleBuilder;
use ephemeral_rollups_pinocchio::types::DelegateConfig;

use crate::error::WickError;

/// Instruction-data buffer for the serialized magic intent bundle.
const INTENT_BUNDLE_DATA_BUF_SIZE: usize = 512;

/// Delegate the guard PDA to the Ephemeral Rollup.
///
/// Account layout (mirrors `delegate_account`):
///   0. `[WRITE, SIGNER]` payer / initializer
///   1. `[WRITE]` guard PDA to delegate
///   2. `[]` owner program (this program)
///   3. `[WRITE]` delegation buffer PDA
///   4. `[WRITE]` delegation record PDA
///   5. `[WRITE]` delegation metadata PDA
///   6. `[]` delegation program
///   7. `[]` system program
///   8. `[optional]` ER validator to target
///
/// `data` = `[bump]`, the guard PDA bump.
pub fn process_delegate(
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let bump = *data.first().ok_or(WickError::InvalidInstruction)?;

    let [payer, guard, owner_program, buffer, record, metadata, _delegation_program, system_program, rest @ ..] =
        accounts
    else {
        return Err(WickError::InvalidInstruction.into());
    };

    let validator = rest.first().map(|account| *account.address());
    let config = DelegateConfig {
        validator,
        ..Default::default()
    };

    delegate_account(
        &[
            payer,
            guard,
            owner_program,
            buffer,
            record,
            metadata,
            system_program,
        ],
        &[GUARD_SEED],
        bump,
        config,
    )
}

/// Commit the guard state to the base layer and undelegate it.
///
/// Account layout:
///   0. `[WRITE, SIGNER]` payer / initializer
///   1. `[WRITE]` guard PDA
///   2. `[]` magic program
///   3. `[]` magic context
pub fn process_commit_and_undelegate(accounts: &[AccountView]) -> ProgramResult {
    let [payer, guard, magic_program, magic_context] = accounts else {
        return Err(WickError::InvalidInstruction.into());
    };

    if !payer.is_signer() {
        return Err(WickError::MissingOwnerAuthority.into());
    }

    let mut data_buf = [0u8; INTENT_BUNDLE_DATA_BUF_SIZE];
    MagicIntentBundleBuilder::new(*payer, *magic_context, *magic_program)
        .commit_and_undelegate(&[*guard])
        .build_and_invoke(&mut data_buf)
}

/// Commit the guard state to the base layer, keeping the guard delegated.
///
/// Account layout:
///   0. `[WRITE, SIGNER]` payer / initializer
///   1. `[WRITE]` guard PDA
///   2. `[]` magic program
///   3. `[]` magic context
pub fn process_commit(accounts: &[AccountView]) -> ProgramResult {
    let [payer, guard, magic_program, magic_context] = accounts else {
        return Err(WickError::InvalidInstruction.into());
    };

    if !payer.is_signer() {
        return Err(WickError::MissingOwnerAuthority.into());
    }

    let mut data_buf = [0u8; INTENT_BUNDLE_DATA_BUF_SIZE];
    MagicIntentBundleBuilder::new(*payer, *magic_context, *magic_program)
        .commit(&[*guard])
        .build_and_invoke(&mut data_buf)
}

/// Undelegation callback invoked by the Delegation Program once the ER session
/// ends. Reverts ownership of the guard PDA back to this program.
///
/// Account layout:
///   0. `[WRITE]` delegated account (guard PDA)
///   1. `[WRITE, SIGNER]` undelegation buffer
///   2. `[WRITE, SIGNER]` payer
///   3. `[]` system program
pub fn process_undelegation_callback(
    program_id: &pinocchio::Address,
    accounts: &[AccountView],
    ix_data: &[u8],
) -> ProgramResult {
    let [delegated, buffer, payer, _system_program, ..] = accounts else {
        return Err(WickError::InvalidInstruction.into());
    };
    undelegate(delegated, program_id, buffer, payer, ix_data)
}

/// Seed used to derive the guard PDA: `b"guard" || owner_pubkey || bump`.
const GUARD_SEED: &[u8] = b"guard";
