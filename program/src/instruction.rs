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
    /// Owner confirms the pending owner-signed venue instruction, committing the
    /// expected nonce (§8.4 CoSigned / §8.7 Jupiter). Lands the Jupiter
    /// safety-net on L1 with the owner's signature (§3 Phase-3).
    ConfirmYes = 9,
    /// Create the singleton RouteConfig PDA (kill-switch authority + pause flag).
    InitRouteConfig = 10,
    /// Close a guard (owner only): refund rent and zero the account so the
    /// owner can re-init. The guard PDA is a pure function of `b"guard" ||
    /// owner`, so without this an account that no longer decodes is a
    /// permanent tombstone at the only address that owner will ever get, with
    /// its rent unrecoverable.
    CloseGuard = 11,
    /// Rotate the RouteConfig kill-switch authority (current authority only).
    /// Without it the authority written at `InitRouteConfig` is permanent.
    SetRouteAuthority = 12,
    /// Reconcile the guard's model of the position against the venue's own
    /// account (permissionless — anyone may pay the fee to keep a guard honest).
    /// On a Drift venue this reads the delegated user's `PerpPosition` for the
    /// pinned market. Divergence is a fail-closed state: autonomous execution
    /// is blocked until the snapshot converges again (§8.3).
    ReconcileVenue = 13,
    /// Create the 2-of-2 margin wallet PDA (`b"margin" || venue_owner`) that
    /// backs `TopUp` with real lamports. One-time, payer-funded.
    InitMarginWallet = 14,
    /// Move lamports from the owner's wallet into the margin-wallet PDA. The
    /// wallet's lamport balance must stay at or above its rent-exempt minimum
    /// plus the recorded `balance` (§8.5).
    FundMarginWallet = 15,
    /// Withdraw lamports out of the margin wallet. 2-of-2: owner + co_authority
    /// must both sign, and the same rent-exempt invariant holds on the way out.
    WithdrawMarginWallet = 16,
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
            9 => Some(Self::ConfirmYes),
            10 => Some(Self::InitRouteConfig),
            11 => Some(Self::CloseGuard),
            12 => Some(Self::SetRouteAuthority),
            13 => Some(Self::ReconcileVenue),
            14 => Some(Self::InitMarginWallet),
            15 => Some(Self::FundMarginWallet),
            16 => Some(Self::WithdrawMarginWallet),
            _ => None,
        }
    }
}
