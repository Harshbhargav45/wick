//! Instruction discriminators (first byte of instruction `data`).

/// Instruction discriminators for the Wick guard program.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WickInstruction {
    /// Initialize a PositionGuard + its 2-of-2 margin wallet.
    InitGuard = 0,
    /// Deposit collateral into the guard margin wallet (owner only).
    DepositMargin = 1,
    /// Withdraw collateral (owner + co_authority together) — §8.5.
    WithdrawMargin = 2,
    /// Pause / resume the whole program (route-config authority). Defined,
    /// wired in a later phase alongside global state.
    SetPaused = 3,
}

impl WickInstruction {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(Self::InitGuard),
            1 => Some(Self::DepositMargin),
            2 => Some(Self::WithdrawMargin),
            3 => Some(Self::SetPaused),
            _ => None,
        }
    }
}