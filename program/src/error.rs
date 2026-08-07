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
}

impl From<WickError> for ProgramError {
    fn from(e: WickError) -> Self {
        ProgramError::Custom(e as u32)
    }
}