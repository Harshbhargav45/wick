//! Error types for the Wick guard program.

use pinocchio::error::ProgramError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WickError {
    /// Invalid instruction discriminator.
    InvalidInstruction = 0x0,
    /// Account owner is not this program.
    WrongAccountOwner = 0x1,
    /// PDA seed derivation did not match.
    InvalidPda = 0x2,
    /// RouteConfig already initialized.
    AlreadyInitialized = 0x3,
    /// RouteConfig not initialized.
    NotInitialized = 0x4,
    /// The co_authority signer is missing for this action.
    MissingCoAuthority = 0x5,
    /// The user owner signer is missing for this action.
    MissingOwnerAuthority = 0x6,
    /// A required signer signed the wrong key.
    SignerKeyMismatch = 0x7,
    /// Fixed-point math overflowed an intermediate.
    MathOverflow = 0x8,
    /// A sourced position can no longer reach its safety buffer.
    CannotReachSafeBuffer = 0x9,
    /// The action exceeds the venue policy cap.
    OverPolicyCap = 0xa,
    /// Replayed or stale nonce.
    Replay = 0xb,
    /// User of a signer mismatch - signer flag set but not a signer account.
    Unauthorized = 0xc,
    /// The venue adapter has no way to execute the selected action yet.
    UnsupportedVenueAction = 0xd,
    /// The venue CPI call failed inside the adapter.
    VenueCpi = 0xe,
    /// No owner-signed instruction is pending that the owner can confirm.
    NoPendingConfirm = 0xf,
    /// The program is paused via RouteConfig kill-switch.
    Paused = 0x10,
    /// A tick nonce that skips ahead of `state.nonce + 1` — rejects the
    /// cranker-DoS where a large nonce bakes every future tick as a replay.
    NonceOutOfOrder = 0x11,
    /// The guard holds a pending action, but this venue/action pair produces no
    /// owner-signed instruction for the owner to confirm. On `VENUE_JUPITER`,
    /// `TakeProfit` and `PartialClose` both build one (§8.4/§8.7); a `TopUp` or
    /// an escalation is advisory and must be resolved at the venue directly.
    ConfirmUnsupportedForVenue = 0x12,
    /// A `ReconcileVenue` whose nonce does not strictly exceed the stored one.
    /// Reconciliation is permissionless, so a replayed transaction would
    /// otherwise re-apply an old venue snapshot over a newer one.
    ReconcileStale = 0x13,
    /// The guard's model of the position is outside tolerance of what the venue
    /// reports. Fail-closed: autonomous execution stays blocked until the owner
    /// resolves it with `UpdatePosition`.
    ReconcileDiverged = 0x14,
    /// The supplied venue position account is not the one this guard watches —
    /// it does not re-derive from the guard's own venue_owner/sub-account, or
    /// it is not owned by the venue program.
    VenueAccountMismatch = 0x15,
    /// The margin wallet cannot cover the requested debit, or its lamport
    /// balance no longer backs the credited `balance` it claims to hold.
    InsufficientMarginWallet = 0x16,
    /// The supplied margin wallet is not `b"margin" || venue_owner` for this
    /// guard. Accepting a foreign wallet would let collateral be credited from
    /// value the guard's owner does not control.
    MarginWalletMismatch = 0x17,
    /// A defensive close cannot be built for the current state — most often the
    /// build request is past its TTL, so landing it would act on a stale level.
    DefensiveCloseUnavailable = 0x18,
}

impl From<WickError> for ProgramError {
    fn from(e: WickError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
