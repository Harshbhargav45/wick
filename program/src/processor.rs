//! Instruction dispatch and handlers.

use pinocchio::error::ProgramResult;
use pinocchio::{AccountView, Address};

use crate::error::WickError;
use crate::instruction::WickInstruction;

pub fn process_instruction(
    _program_id: &Address,
    _accounts: &mut [AccountView],
    data: &[u8],
) -> ProgramResult {
    let Some(discriminator_byte) = data.first().copied() else {
        return Err(WickError::InvalidInstruction.into());
    };
    let Some(ix) = WickInstruction::from_byte(discriminator_byte) else {
        return Err(WickError::InvalidInstruction.into());
    };

    match ix {
        WickInstruction::SetPaused => Err(WickError::InvalidInstruction.into()),
        _ => {
            // Phase 1: handlers for init/deposit/withdraw are implemented once
            // the raw AccountView byte access is wired against the account
            // layout in state.rs (§8.5's check is unit-tested here).
            Err(WickError::InvalidInstruction.into())
        }
    }
}

/// §8.5 The 2-of-2 authority check as a pure function so it can be unit-tested
/// without an instruction context. `user` must be a signer AND match the
/// wallet owner; `guard_pda` must be a signer AND match the co_authority.
pub fn validate_withdraw(
    user_is_signer: bool,
    user_key: &Address,
    owner: &Address,
    co_auth_is_signer: bool,
    co_auth_key: &Address,
    co_authority: &Address,
) -> Result<(), WickError> {
    if !user_is_signer || user_key != owner {
        return Err(WickError::MissingOwnerAuthority);
    }
    if !co_auth_is_signer || co_auth_key != co_authority {
        return Err(WickError::MissingCoAuthority);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pinocchio::Address;

    fn addr(seed: u8) -> Address {
        Address::from([seed; 32])
    }

    #[test]
    fn withdraw_requires_both_signatures() {
        let owner = addr(1);
        let co = addr(2);

        // user-only fails
        assert_eq!(
            validate_withdraw(true, &owner, &owner, false, &co, &co).unwrap_err(),
            WickError::MissingCoAuthority
        );
        // co-authority-only fails
        assert_eq!(
            validate_withdraw(false, &owner, &owner, true, &co, &co).unwrap_err(),
            WickError::MissingOwnerAuthority
        );
        // signer flag set but wrong pubkey fails
        assert_eq!(
            validate_withdraw(true, &owner, &co, true, &co, &co).unwrap_err(),
            WickError::MissingOwnerAuthority
        );
        // both correct succeeds
        assert!(validate_withdraw(true, &owner, &owner, true, &co, &co).is_ok());
    }
}