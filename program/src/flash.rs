//! FlashTrade venue adapter (§8.7).
//!
//! Builds and performs the Flash perpetuals `close_position` CPI that Wick's
//! guard executes when a protection action fires. Flash is an Anchor program
//! running pool-to-peer on Solana; FlashTrade V2 additionally routes through
//! MagicBlock's Ephemeral Rollup for sub-50ms execution (500x leverage, 2bp).
//!
//! This module encodes the exact instruction data and account layout taken from
//! Flash's audited `flash-perpetuals` source (`close_position.rs`):
//!
//! * Anchor 8-byte discriminator `sha256("global:close_position")[..8]`.
//! * `ClosePositionParams { price: u64 }` (max acceptable exit price, in the
//!   custody's price exponent; the guard passes its breach-detection price).
//! * Account order (matching `#[derive(Accounts)]`):
//!   0. `owner`            [`Signer`, WRITE]
//!   1. `receiving_account` [WRITE] — user's collateral token account
//!   2. `transfer_authority` [read] — `b"transfer_authority"` PDA
//!   3. `perpetuals`        [read] — `b"perpetuals"` PDA
//!   4. `pool`              [WRITE] — `b"pool" || pool.name` PDA
//!   5. `position`          [WRITE, close] — `b"position"||owner||pool||custody||[side]`
//!   6. `custody`           [WRITE]
//!   7. `custody_oracle_account`
//!   8. `collateral_custody [WRITE]
//!   9. `collateral_custody_oracle_account`
//!   10. `collateral_custody_token_account` [WRITE]
//!   11. `token_program`

use pinocchio::cpi::invoke_signed_with_bounds;
use pinocchio::instruction::{InstructionAccount, InstructionView};
use pinocchio::{AccountView, Address, ProgramResult};

use crate::error::WickError;

/// Flash Perpetuals program ID (mainnet-beta).
pub const FLASH_PROGRAM_ID: Address =
    Address::new_from_array([
        212, 236, 82, 74, 222, 71, 209, 50, 127, 252, 246, 137, 90, 104, 93, 148, 41, 240, 55,
        144, 196, 35, 87, 71, 243, 123, 215, 163, 221, 165, 30, 221,
    ]);

/// Anchor discriminator of `global:close_position`.
const CLOSE_POSITION_DISCRIMINATOR: [u8; 8] = [123, 134, 81, 0, 49, 68, 98, 98];

/// Number of accounts in Flash's `close_position`.
const CLOSE_POSITION_ACCOUNT_COUNT: usize = 12;

/// Maximum acceptable exit price, in the custody token's price scale.
#[derive(Clone, Copy, Debug)]
pub struct ClosePositionParams {
    pub price: u64,
}

impl ClosePositionParams {
    /// Serialize Anchor `ClosePositionParams`: 8-byte discriminator + LE u64.
    pub fn try_to_data(&self) -> Result<[u8; 16], WickError> {
        let mut data = [0u8; 16];
        data[..8].copy_from_slice(&CLOSE_POSITION_DISCRIMINATOR);
        data[8..16].copy_from_slice(&self.price.to_le_bytes());
        Ok(data)
    }
}

/// The accounts required by Flash's `close_position`, in the exact order the
/// Anchor context expects.
pub struct ClosePositionAccounts<'a> {
    pub owner: &'a AccountView,
    pub receiving_account: &'a AccountView,
    pub transfer_authority: &'a AccountView,
    pub perpetuals: &'a AccountView,
    pub pool: &'a AccountView,
    pub position: &'a AccountView,
    pub custody: &'a AccountView,
    pub custody_oracle_token: &'a AccountView,
    pub collateral_custody: &'a AccountView,
    pub collateral_custody_oracle: &'a AccountView,
    pub collateral_custody_token_account: &'a AccountView,
    pub token_program: &'a AccountView,
}

impl<'a> ClosePositionAccounts<'a> {
    /// Build the account meta slice for the CPI.
    fn metas(&self) -> [InstructionAccount<'a>; CLOSE_POSITION_ACCOUNT_COUNT] {
        let writable = |a: &'a AccountView| InstructionAccount::writable(a.address());
        let readonly = |a: &'a AccountView| InstructionAccount::readonly(a.address());
        [
            InstructionAccount::writable_signer(self.owner.address()),
            writable(self.receiving_account),
            readonly(self.transfer_authority),
            readonly(self.perpetuals),
            writable(self.pool),
            writable(self.position),
            writable(self.custody),
            readonly(self.custody_oracle_token),
            writable(self.collateral_custody),
            readonly(self.collateral_custody_oracle),
            writable(self.collateral_custody_token_account),
            readonly(self.token_program),
        ]
    }

    /// CPI into Flash `close_position`.
    ///
    /// The `owner` account signs this instruction: Flash requires the position
    /// owner's signature on every state change (no delegated authority exists),
    /// so this adapter is used in the *co-signed* protection tier. It never
    /// claims autonomous execution on Flash.
    pub fn invoke(&self, params: &ClosePositionParams) -> ProgramResult {
        let data = params.try_to_data()?;
        let metas = self.metas();
        let instruction = InstructionView {
            program_id: &FLASH_PROGRAM_ID,
            data: &data,
            accounts: &metas,
        };
        let account_views = [
            self.owner,
            self.receiving_account,
            self.transfer_authority,
            self.perpetuals,
            self.pool,
            self.position,
            self.custody,
            self.custody_oracle_token,
            self.collateral_custody,
            self.collateral_custody_oracle,
            self.collateral_custody_token_account,
            self.token_program,
        ];
        // No guard PDA signs here — the venue requires the owner's signature,
        // which must already be present in the transaction.
        invoke_signed_with_bounds::<CLOSE_POSITION_ACCOUNT_COUNT>(&instruction, &account_views, &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_position_data_layout() {
        let params = ClosePositionParams { price: 100_000_000 };
        let data = params.try_to_data().unwrap();
        assert_eq!(&data[..8], &CLOSE_POSITION_DISCRIMINATOR);
        assert_eq!(
            u64::from_le_bytes(data[8..16].try_into().unwrap()),
            100_000_000
        );
    }

    #[test]
    fn discriminator_matches_anchor() {
        // `sha256("global:close_position")[..8]` — pinned from the audited source.
        assert_eq!(
            CLOSE_POSITION_DISCRIMINATOR,
            [123, 134, 81, 0, 49, 68, 98, 98]
        );
    }

    #[test]
    fn program_id_is_flash() {
        // `FLASH6Lo6h3iasJKWDs2F8TkW2UKf3s15C8PMGuVfgBn`
        assert_eq!(
            FLASH_PROGRAM_ID.to_bytes(),
            [
                212, 236, 82, 74, 222, 71, 209, 50, 127, 252, 246, 137, 90, 104, 93, 148, 41, 240,
                55, 144, 196, 35, 87, 71, 243, 123, 215, 163, 221, 165, 30, 221
            ]
        );
    }
}