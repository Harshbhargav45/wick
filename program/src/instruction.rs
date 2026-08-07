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
    /// Delegate the guard PDA to the MagicBlock Ephemeral Rollup (§8.6).
    Delegate = 4,
    /// Commit guard state back to the base layer and undelegate (§8.6).
    CommitAndUndelegate = 5,
    /// Commit guard state to the base layer, keeping the guard delegated.
    Commit = 6,
    /// A price tick from the ER oracle — runs the §7.2 critical path:
    /// staleness → health → nonce → caps → action selection → dispatch.
    OnPriceTick = 7,
    /// Record the watched position's snapshot (collateral, size, entry). Called
    /// by the owner when the position is opened so the guard has real state.
    UpdatePosition = 8,
}

impl WickInstruction {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(Self::InitGuard),
            1 => Some(Self::DepositMargin),
            2 => Some(Self::WithdrawMargin),
            3 => Some(Self::SetPaused),
            4 => Some(Self::Delegate),
            5 => Some(Self::CommitAndUndelegate),
            6 => Some(Self::Commit),
            7 => Some(Self::OnPriceTick),
            8 => Some(Self::UpdatePosition),
            _ => None,
        }
    }
}
