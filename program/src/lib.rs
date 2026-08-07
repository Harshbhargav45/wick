//! Wick — autonomous onchain liquidation-protection layer for Solana perps.
//!
//! Phase 1: the Pinocchio guard program (no venues wired yet). This crate
//! implements the account layout, the overflow-safe fixed-point health engine
//! (§8.1), the action selector (§8.2), the bounded partial-close solver (§8.3),
//! two-regime authority dispatch (§8.4), and the 2-of-2 co-authority checks
//! (§8.5). Venue adapters (FlashTrade / Jupiter) land in later phases.

#![no_std]

pub mod account;
pub mod delegation;
pub mod error;
pub mod instruction;
pub mod processor;
pub mod state;

#[cfg(not(feature = "no-entrypoint"))]
mod entrypoint {
    use pinocchio::{default_allocator, program_entrypoint};

    program_entrypoint!(crate::processor::process_instruction);

    default_allocator!();
}

#[cfg(all(not(feature = "no-entrypoint"), not(test)))]
mod panic_handler {
    use pinocchio::nostd_panic_handler;
    nostd_panic_handler!();
}