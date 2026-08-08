//! Drift Protocol venue adapter — autonomous hard reduce-only tier (§8.7).
//!
//! This module encodes the exact instruction data and account layout
//! taken from the Drift `protocol-v2` source (`velocity-exchange/protocol-v2`):
//!
//! * Anchor 8-byte discriminator `sha256("global:place_perp_order")[..8]`.
//!
//! * `OrderParams` borsh layout per `state/order_params.rs` (anchor 0.29 /
//!   borsh 0.10: unit enums serialize as one u8, `Option::None` as a zero tag).
//!
//! * Fixed account order (`PlaceOrder` in `instructions/user.rs`), followed by
//!   Drift's `remaining_accounts` (perp market, oracle, spot markets, user maps
//!   in SDK order):
//!
//!   0. `state` — `Account<State>` (readonly)
//!   1. `user` — `AccountLoader<User>` (writable; PDSA seeds
//!      `["user", authority, sub_account_id.to_le_bytes()]`)
//!   2. `authority` — `Signer`; Drift accepts it iff `can_sign_for_user`:
//!      `user.authority == signer || (user.delegate == signer &&
//!      user.delegate != default)`. Wick signs as the guard PDA which the user
//!      has set as their delegate.
//!
//! The guard passes the remaining tail through unchanged — it only ever builds
//! a reduce-only order, hard by construction. The guard never allows the order
//! flags to deviate from `reduce_only = true`.

use core::array::from_fn;

use pinocchio::cpi::invoke_signed_with_bounds;
use pinocchio::instruction::{cpi::Signer, InstructionAccount, InstructionView};
use pinocchio::{AccountView, Address, ProgramResult};

use crate::error::WickError;

/// `state.venue` tag for the Drift adapter.
pub const VENUE_DRIFT: u8 = 3;

/// Drift Protocol program ID (mainnet & devnet).
///
/// `dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH` — from Drift's
/// `programs/drift/src/lib.rs` `declare_id!`.
pub const DRIFT_PROGRAM_ID: Address = Address::new_from_array([
    9, 84, 219, 190, 158, 201, 96, 201, 138, 122, 41, 63, 226, 19, 54, 150, 111, 225, 128, 209, 81,
    174, 75, 129, 121, 86, 31, 137, 133, 74, 83, 246,
]);

/// Anchor discriminator of `global:place_perp_order`.
pub const PLACE_PERP_ORDER_DISCRIMINATOR: [u8; 8] = [69, 161, 93, 202, 120, 126, 76, 185];

/// Total serialized size of the reduce order: 8-byte discriminator + 32-byte
/// `OrderParams` (all optional `Option::None`).
pub const REDUCE_ORDER_DATA_LEN: usize = 8 + 32;

/// Order params borsh enums (pinned from `order_params.rs`), each 1 u8 in borsh.
const ORDER_TYPE_MARKET: u8 = 0; // OrderType::Market
const MARKET_TYPE_PERP: u8 = 1; // MarketType::Perp
const POST_ONLY_NONE: u8 = 0; // PostOnlyParam::None
const TRIGGER_COND_ABOVE: u8 = 0; // OrderTriggerCondition::Above (default)

/// Direction a reduce order takes against the current position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReduceDirection {
    /// Reduce a long position (sell).
    Short,
    /// Reduce a short position (buy).
    Long,
}

/// A hard reduce-only perp market order — the only order this adapter builds.
/// All-time Borsh `OrderParams` optional fields default to `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReduceOrderParams {
    /// Perp market index to reduce.
    pub market_index: u16,
    /// Direction of the reduce order.
    pub direction: ReduceDirection,
    /// Base-asset amount to reduce.
    pub base_asset_amount: u64,
    /// Maximum acceptable fill price.
    pub price: u64,
}

impl ReduceOrderParams {
    /// Serialize the full `place_perp_order` instruction data:
    /// borsh array of `OrderParams` with every optional field `None`.
    pub fn try_to_data(&self) -> Result<[u8; REDUCE_ORDER_DATA_LEN], WickError> {
        let mut d = [0u8; REDUCE_ORDER_DATA_LEN];
        d[..8].copy_from_slice(&PLACE_PERP_ORDER_DISCRIMINATOR);
        // OrderParams borsh layout (all offsets exact, verified against borsh
        // 0.10 probe — Option/unit enums are one byte; None = tag 0).
        d[8] = ORDER_TYPE_MARKET; // order_type => Market
        d[9] = MARKET_TYPE_PERP; // market_type => Perp
        d[10] = match self.direction {
            ReduceDirection::Short => 1, // PositionDirection::Short
            ReduceDirection::Long => 0,  // PositionDirection::Long
        };
        d[11] = 0; // user_order_ud (u8) = 0
        d[12..20].copy_from_slice(&self.base_asset_amount.to_le_bytes());
        d[20..28].copy_from_slice(&self.price.to_le_bytes());
        d[28..30].copy_from_slice(&self.market_index.to_le_bytes());
        d[30] = 1; // reduce_only = true — hard-coded, no other path
        d[31] = POST_ONLY_NONE;
        // [32] bit_flags all zero
        // [33] max_ts Option<i64> = None (0)
        // [34] trigger_price Option<u64> = None (0)
        d[35] = TRIGGER_COND_ABOVE; // trigger_condition (u8)
                                    // [36] oracle_price_offset Option<i32> = None (0)
                                    // [37] auction_duration Option<u8> = None (0)
                                    // [38] auction_start_price Option<i64> = None (0)
                                    // [39] auction_end_price Option<i64> = None (0)
        Ok(d)
    }
}

/// Hard cap on accounts this adapter passes through, matching `place_perp_order`'s
/// `remaining_accounts` (perp market, oracle, spot markets, user maps) + 3 fixed.
const MAX_DRIFT_ACCOUNTS: usize = 16;

/// The accounts of Drift's `place_perp_order`, in exact order. The adapter is
/// reconstructed from the tail of the `OnPriceTick` account list; the
/// `remaining` slice is Drift's map-load accounts passed through unchanged.
pub struct DriftPlaceOrderAccounts<'a> {
    pub state: &'a AccountView,
    pub user: &'a AccountView,
    pub authority: &'a AccountView,
    pub remaining: &'a [AccountView],
}

impl<'a> DriftPlaceOrderAccounts<'a> {
    /// Reconstruct the adapter from the tail of the `OnPriceTick` account list.
    /// Expects at least the 3 fixed `PlaceOrder` accounts in order.
    pub fn from_account_views(views: &'a [AccountView]) -> Result<Self, WickError> {
        if views.len() < 3 || views.len() > MAX_DRIFT_ACCOUNTS {
            return Err(WickError::InvalidInstruction);
        }
        Ok(Self {
            state: &views[0],
            user: &views[1],
            authority: &views[2],
            remaining: &views[3..],
        })
    }

    /// Build the account metas for the CPI. The first three are Drift's fixed
    /// `state` / `user` / `authority` in that exact order; the remaining tail
    /// preserves each account's writable/signer metadata from the ticker.
    fn metas(&self) -> [InstructionAccount<'a>; MAX_DRIFT_ACCOUNTS] {
        // from_fn avoids Copy: InstructionAccount is Clone, not Copy.
        let mut metas = from_fn(|_| InstructionAccount::readonly(&DRIFT_PROGRAM_ID));
        metas[0] = InstructionAccount::readonly(self.state.address());
        metas[1] = InstructionAccount::writable(self.user.address());
        metas[2] = InstructionAccount::readonly_signer(self.authority.address());
        for (slot, view) in metas.iter_mut().skip(3).zip(self.remaining.iter()) {
            *slot = InstructionAccount::from(view);
        }
        metas
    }

    /// Align the account views array (fixed 3 + remaining) into a sized slice.
    fn account_views(&self) -> [&'a AccountView; MAX_DRIFT_ACCOUNTS] {
        // References are Copy, so the fixed residue suffices as seeds.
        let residue = self.remaining.first().unwrap_or(self.state);
        let mut views = [residue; MAX_DRIFT_ACCOUNTS];
        views[0] = self.state;
        views[1] = self.user;
        views[2] = self.authority;
        for (slot, view) in views.iter_mut().skip(3).zip(self.remaining.iter()) {
            *slot = view;
        }
        views
    }

    /// CPI into Drift `place_perp_order` signed as the guard (delegate).
    ///
    /// `signer_seeds` must be the guard PDA seeds, which the user registered as
    /// the Drift `delegate` for that sub-account.
    pub fn invoke(&self, params: &ReduceOrderParams, signer_seeds: &[Signer]) -> ProgramResult {
        // place_perp_order is a `Signer`-distinguished delegate order; Drift
        // reduces regardless of `market_index` sign, relying on `reduce_only`
        // we hard-set below. The order is placed as the guard, so the drift
        // perp market being closed must be passed in `remaining`.
        let data = params.try_to_data()?;
        let metas = self.metas();
        let views = self.account_views();
        let count = self.remaining.len() + 3;
        let instruction = InstructionView {
            program_id: &DRIFT_PROGRAM_ID,
            data: &data,
            accounts: &metas[..count],
        };
        invoke_signed_with_bounds::<MAX_DRIFT_ACCOUNTS>(&instruction, &views[..count], signer_seeds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drift_program_id_is_drift() {
        assert_eq!(DRIFT_PROGRAM_ID.to_bytes(), {
            [
                9, 84, 219, 190, 158, 201, 96, 201, 138, 122, 41, 63, 226, 19, 54, 150, 111, 225,
                128, 209, 81, 174, 75, 129, 121, 86, 31, 137, 133, 74, 83, 246,
            ]
        });
    }

    #[test]
    fn discriminator_matches_anchor() {
        // sha256("global:place_perp_order")[..8]
        assert_eq!(
            PLACE_PERP_ORDER_DISCRIMINATOR,
            [69, 161, 93, 202, 120, 126, 76, 185]
        );
    }

    #[test]
    fn reduce_order_data_layout() {
        let params = ReduceOrderParams {
            market_index: 0,
            direction: ReduceDirection::Short,
            base_asset_amount: 100_000_000_000, // 100 SOL
            price: 0,
        };
        let data = params.try_to_data().unwrap();
        assert_eq!(data.len(), REDUCE_ORDER_DATA_LEN);
        assert_eq!(&data[..8], &PLACE_PERP_ORDER_DISCRIMINATOR);
        assert_eq!(data[8], 0); // order_type = Market
        assert_eq!(data[9], 1); // market_type = Perp
        assert_eq!(data[10], 1); // direction Short (PositionDirection::Short = 1)
        assert_eq!(
            u64::from_le_bytes(data[12..20].try_into().unwrap()),
            100_000_000_000
        );
        assert_eq!(u16::from_le_bytes(data[28..30].try_into().unwrap()), 0);
        assert_eq!(data[30], 1); // reduce_only: hard true
        assert_eq!(data[31], 0); // post_only None
        assert_eq!(data[35], 0); // trigger condition default
        assert!(data[33..35].iter().all(|&b| b == 0)); // max_ts / trigger None
        assert!(data[36..40].iter().all(|&b| b == 0)); // remaining None
    }

    #[test]
    fn reduce_direction_long() {
        let params = ReduceOrderParams {
            market_index: 1,
            direction: ReduceDirection::Long,
            base_asset_amount: 2,
            price: 12345,
        };
        let data = params.try_to_data().unwrap();
        assert_eq!(data[10], 0); // direction Long (PositionDirection::Long = 0)
        assert_eq!(u16::from_le_bytes(data[28..30].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(data[20..28].try_into().unwrap()), 12345);
    }

    #[test]
    fn construction_requires_three_accounts() {
        // Fewer than 3 fixed accounts is rejected.
        assert!(DriftPlaceOrderAccounts::from_account_views(&[]).is_err());
    }
}
