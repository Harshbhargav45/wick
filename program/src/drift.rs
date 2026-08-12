//! Drift venue adapter — autonomous hard reduce-only tier (§8.7).
//!
//! This module encodes the exact instruction data and account layout taken
//! from the live successor of Drift Protocol v2 (`velocity-exchange/protocol-v2`),
//! deployed as **Velocity** (`vELoC1...`). The `dRifty...` program was
//! decommissioned after the 2026 exploit and its on-chain binary now only
//! handles withdrawals; the only live program that still executes
//! `place_perp_order` is Velocity (same program ABI, new program ID).
//!
//! * Anchor 8-byte discriminator `sha256("global:place_perp_order")[..8]`.
//!
//! * `OrderParams` borsh layout per `state/order_params.rs` (anchor 0.29 /
//!   borsh 0.10: unit enums serialize as one u8, `Option::None` as a zero tag).
//!   Velocity's `OrderParams` gained two trailing fields over Drift's —
//!   `builder_idx: Option<u8>`, `builder_fee_tenth_bps: Option<u16>` — so the
//!   all-`None` params are 34 bytes, not 32.
//!
//! * Fixed account order (`PlaceOrder` in `instructions/user.rs`), followed by
//!   the program's `remaining_accounts` (perp market, oracles, spot market in
//!   SDK order — `getRemainingAccounts` emits oracles, then spot markets, then
//!   perp markets):
//!
//!   0. `state` — `Account<State>` (readonly)
//!   1. `user` — `AccountLoader<User>` (writable; PDA seeds
//!      `["user", authority, sub_account_id.to_le_bytes()]`)
//!   2. `authority` — `Signer`; the program accepts it iff `can_sign_for_user`:
//!      `user.authority == signer || (user.delegate == signer &&
//!      user.delegate != default)`. Wick signs as the guard PDA which the user
//!      has set as their delegate. (`User.delegate` sits at data offset 40:
//!      8-byte Anchor discriminator + `authority` Pubkey.)
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

/// Drift venue program ID — the **live** successor to the decommissioned
/// `dRifty...` program.
///
/// `vELoC1audYbSYVRXn1vPaV8Axoa9oU6BYmNGZZBDZ1P` — from Velocity's
/// `programs/drift/src/lib.rs` `declare_id!`.
pub const DRIFT_PROGRAM_ID: Address = Address::new_from_array([
    13, 162, 222, 50, 93, 130, 241, 222, 120, 205, 77, 177, 103, 33, 15, 103, 45, 147, 250, 167,
    129, 184, 165, 217, 84, 183, 159, 1, 88, 249, 227, 150,
]);

/// Anchor discriminator of `global:place_perp_order`.
pub const PLACE_PERP_ORDER_DISCRIMINATOR: [u8; 8] = [69, 161, 93, 202, 120, 126, 76, 185];

/// Total serialized size of the reduce order: 8-byte discriminator + 34-byte
/// `OrderParams` (all optional `Option::None`). Velocity's `OrderParams` adds
/// `builder_idx: Option<u8>` and `builder_fee_tenth_bps: Option<u16>` after
/// Drift's 32-byte layout.
pub const REDUCE_ORDER_DATA_LEN: usize = 8 + 34;

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
/// Every optional Borsh `OrderParams` field is serialized as `None`.
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
        d[11] = 0; // user_order_id (u8) = 0
        d[12..20].copy_from_slice(&self.base_asset_amount.to_le_bytes());
        d[20..28].copy_from_slice(&self.price.to_le_bytes());
        d[28..30].copy_from_slice(&self.market_index.to_le_bytes());
        d[30] = 1; // reduce_only = true — hard-coded, no other path
        d[31] = POST_ONLY_NONE;
        // [32] bit_flags all zero
        // [33] max_ts Option<i64> = None (0)
        // [34] trigger_price Option<u64> = None (0)
        d[35] = TRIGGER_COND_ABOVE; // trigger_condition (u8)
                                    // [36] oracle_price_offset Option<i64> = None (0)
                                    // [37] auction_duration Option<u8> = None (0)
                                    // [38] auction_start_price Option<i64> = None (0)
                                    // [39] auction_end_price Option<i64> = None (0)
                                    // [40] builder_idx Option<u8> = None (0) — Velocity-only
                                    // [41] builder_fee_tenth_bps Option<u16> = None (0) — Velocity-only
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
        // The order is placed as the guard (the delegate), so the perp market
        // being reduced has to arrive in `remaining`. `reduce_only` is forced
        // by `try_to_data`, so this path can only ever shrink the position.
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

// -------------------------------------------------------------------------
// Read-only position decoder (§8.3 reconciliation)
// -------------------------------------------------------------------------

/// Anchor account discriminator of Velocity's `User` — `sha256("account:User")[..8]`.
/// Pinned against the real mainnet fixture in `tests/fixtures/` (see
/// `tests/real_drift.rs`, which builds a byte-identical synthetic user).
pub const USER_DISCRIMINATOR: [u8; 8] = [0x9f, 0x75, 0x5f, 0xe3, 0xef, 0x97, 0x3a, 0xec];

/// Total `User` account size: 8-byte Anchor discriminator + 4488-byte struct.
pub const USER_SIZE: usize = 4496;

/// `User.perp_positions` — offset of the array, its element stride, and the
/// number of slots. All three are pinned against the mainnet fixture; a wrong
/// offset here would silently read a neighbouring field as a position size,
/// which is exactly the class of error reconciliation exists to catch.
const PERP_POSITIONS_OFFSET: usize = 424;
const PERP_POSITION_STRIDE: usize = 80;
const PERP_POSITION_COUNT: usize = 8;

/// `PerpPosition` field offsets, relative to the position's own start.
const PERP_BASE_ASSET_AMOUNT_OFF: usize = 8;
const PERP_QUOTE_ASSET_AMOUNT_OFF: usize = 16;
const PERP_MARKET_INDEX_OFF: usize = 76;

/// `User.spot_positions` — the quote (collateral) side.
const SPOT_POSITIONS_OFFSET: usize = 104;
const SPOT_POSITION_STRIDE: usize = 40;
const SPOT_POSITION_COUNT: usize = 8;
const SPOT_SCALED_BALANCE_OFF: usize = 0;
const SPOT_MARKET_INDEX_OFF: usize = 32;
const SPOT_BALANCE_TYPE_OFF: usize = 34;
/// `SpotBalanceType::Deposit`. A borrow (1) is negative collateral and is not
/// counted as backing — treating it as a deposit would inflate the health math.
const SPOT_BALANCE_TYPE_DEPOSIT: u8 = 0;
/// Quote spot market index. Velocity, like Drift, pins the quote asset to 0.
const QUOTE_SPOT_MARKET_INDEX: u16 = 0;

/// Velocity's `SpotBalance` precision is 1e9 while the guard carries USD at
/// 1e6 (`state::SCALE`). Deposits are scaled down by this factor; using the
/// raw balance would report collateral a thousand times too large.
const SPOT_BALANCE_TO_SCALE6_DIVISOR: u128 = 1_000;

/// The venue's own view of a watched position, read from its account bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VenuePosition {
    /// Signed base-asset amount: positive long, negative short.
    pub size: i128,
    /// Quote collateral backing the account, in the guard's 6dp USD scale.
    pub collateral: u128,
}

/// Decode the watched `PerpPosition` and the quote collateral out of a
/// Velocity `User` account.
///
/// This is deliberately a *read* of the venue's own bytes rather than a
/// caller-supplied number — the same stance `on_price_tick` takes with the
/// price. A keeper calling `ReconcileVenue` can choose *when* the guard looks
/// at the venue, never *what it sees*.
///
/// A market the user holds no position in decodes as size 0, not an error: a
/// position closed at the venue is a real and important observation, and it is
/// precisely the divergence the guard needs to notice.
pub fn read_user_position(data: &[u8], market_index: u16) -> Result<VenuePosition, WickError> {
    if data.len() < USER_SIZE || data[..8] != USER_DISCRIMINATOR {
        return Err(WickError::VenueAccountMismatch);
    }

    let mut size: i128 = 0;
    for slot in 0..PERP_POSITION_COUNT {
        let base = PERP_POSITIONS_OFFSET + slot * PERP_POSITION_STRIDE;
        let idx = u16::from_le_bytes(
            data[base + PERP_MARKET_INDEX_OFF..base + PERP_MARKET_INDEX_OFF + 2]
                .try_into()
                .map_err(|_| WickError::VenueAccountMismatch)?,
        );
        if idx != market_index {
            continue;
        }
        let base_amt = i64::from_le_bytes(
            data[base + PERP_BASE_ASSET_AMOUNT_OFF..base + PERP_BASE_ASSET_AMOUNT_OFF + 8]
                .try_into()
                .map_err(|_| WickError::VenueAccountMismatch)?,
        );
        // Velocity zeroes a closed position's `market_index` too, so slot 0 of
        // an empty account looks like "market 0, size 0". That decodes to the
        // honest answer — no exposure — so no extra emptiness test is needed.
        let _quote = i64::from_le_bytes(
            data[base + PERP_QUOTE_ASSET_AMOUNT_OFF..base + PERP_QUOTE_ASSET_AMOUNT_OFF + 8]
                .try_into()
                .map_err(|_| WickError::VenueAccountMismatch)?,
        );
        size = base_amt as i128;
        break;
    }

    let mut collateral: u128 = 0;
    for slot in 0..SPOT_POSITION_COUNT {
        let base = SPOT_POSITIONS_OFFSET + slot * SPOT_POSITION_STRIDE;
        let idx = u16::from_le_bytes(
            data[base + SPOT_MARKET_INDEX_OFF..base + SPOT_MARKET_INDEX_OFF + 2]
                .try_into()
                .map_err(|_| WickError::VenueAccountMismatch)?,
        );
        if idx != QUOTE_SPOT_MARKET_INDEX
            || data[base + SPOT_BALANCE_TYPE_OFF] != SPOT_BALANCE_TYPE_DEPOSIT
        {
            continue;
        }
        let scaled = u64::from_le_bytes(
            data[base + SPOT_SCALED_BALANCE_OFF..base + SPOT_SCALED_BALANCE_OFF + 8]
                .try_into()
                .map_err(|_| WickError::VenueAccountMismatch)?,
        );
        collateral = (scaled as u128) / SPOT_BALANCE_TO_SCALE6_DIVISOR;
        break;
    }

    Ok(VenuePosition { size, collateral })
}

/// Re-derive the Velocity user PDA the guard is delegated on and check that the
/// supplied account really is it.
///
/// `ReconcileVenue` is permissionless, so the position account is
/// attacker-supplied. Without this check a keeper could point the guard at any
/// account whose bytes happen to decode, and the guard would adopt a fabricated
/// position as ground truth. Both halves matter: the address must re-derive
/// from *the guard's own* `venue_owner` and sub-account, and the account must
/// be owned by the venue program so its contents are the venue's, not a
/// look-alike the caller wrote themselves.
pub fn verify_user_account(
    account: &AccountView,
    venue_owner: &[u8; 32],
    subaccount_id: u16,
) -> Result<(), WickError> {
    if !account.owned_by(&DRIFT_PROGRAM_ID) {
        return Err(WickError::VenueAccountMismatch);
    }
    let sub = subaccount_id.to_le_bytes();
    // The canonical bump is not stored on the guard, so every bump is tried
    // from the top down — `find_program_address` in reverse. In practice the
    // first candidate matches; the loop exists so a non-canonical-but-valid
    // sub-account cannot lock the guard out of reconciling.
    for bump in (0u8..=255).rev() {
        let Ok(candidate) = Address::create_program_address(
            &[b"user", venue_owner, &sub, &[bump]],
            &DRIFT_PROGRAM_ID,
        ) else {
            continue;
        };
        if account.address() == &candidate {
            return Ok(());
        }
    }
    Err(WickError::VenueAccountMismatch)
}

/// Build a `User` account body matching the real mainnet layout — the same
/// construction `tests/real_drift.rs` uses against the live Velocity BPF,
/// which is what pins these offsets to reality rather than to memory.
///
/// Lives outside the test module because `processor::tests` needs it too: the
/// reconciliation handler's whole job is decoding these bytes, and giving it a
/// second, hand-rolled fixture would let the two drift apart silently.
#[cfg(test)]
pub(crate) fn synthetic_user(
    market_index: u16,
    base_amount: i64,
    scaled_balance: u64,
) -> [u8; USER_SIZE] {
    let mut data = [0u8; USER_SIZE];
    data[..8].copy_from_slice(&USER_DISCRIMINATOR);
    let sp = SPOT_POSITIONS_OFFSET;
    data[sp..sp + 8].copy_from_slice(&scaled_balance.to_le_bytes());
    data[sp + SPOT_MARKET_INDEX_OFF..sp + SPOT_MARKET_INDEX_OFF + 2]
        .copy_from_slice(&0u16.to_le_bytes());
    data[sp + SPOT_BALANCE_TYPE_OFF] = SPOT_BALANCE_TYPE_DEPOSIT;
    let pp = PERP_POSITIONS_OFFSET;
    data[pp + PERP_BASE_ASSET_AMOUNT_OFF..pp + PERP_BASE_ASSET_AMOUNT_OFF + 8]
        .copy_from_slice(&base_amount.to_le_bytes());
    data[pp + PERP_MARKET_INDEX_OFF..pp + PERP_MARKET_INDEX_OFF + 2]
        .copy_from_slice(&market_index.to_le_bytes());
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drift_program_id_is_velocity() {
        assert_eq!(DRIFT_PROGRAM_ID.to_bytes(), {
            [
                13, 162, 222, 50, 93, 130, 241, 222, 120, 205, 77, 177, 103, 33, 15, 103, 45, 147,
                250, 167, 129, 184, 165, 217, 84, 183, 159, 1, 88, 249, 227, 150,
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
        assert!(data[40..42].iter().all(|&b| b == 0)); // velocity builder fields None
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

    #[test]
    fn reads_long_position_and_collateral() {
        // 0.1 base asset long on market 0, 1e15 scaled balance = 1e12 in 6dp.
        let data = synthetic_user(0, 100_000_000, 1_000_000_000_000_000);
        let pos = read_user_position(&data, 0).unwrap();
        assert_eq!(pos.size, 100_000_000);
        assert_eq!(pos.collateral, 1_000_000_000_000);
    }

    #[test]
    fn reads_short_position_as_negative() {
        let data = synthetic_user(3, -250_000_000, 0);
        let pos = read_user_position(&data, 3).unwrap();
        assert_eq!(pos.size, -250_000_000);
    }

    /// A market the user is not in reads as flat. That is the observation that
    /// makes "the owner closed at the venue" detectable at all.
    #[test]
    fn unheld_market_reads_flat() {
        let data = synthetic_user(0, 100_000_000, 0);
        let pos = read_user_position(&data, 9).unwrap();
        assert_eq!(pos.size, 0);
    }

    #[test]
    fn rejects_foreign_account_bytes() {
        // Right size, wrong discriminator — someone else's account.
        let mut data = [0u8; USER_SIZE];
        data[..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(
            read_user_position(&data, 0).unwrap_err(),
            WickError::VenueAccountMismatch
        );
        // Right discriminator, truncated body.
        let mut short = [0u8; 128];
        short[..8].copy_from_slice(&USER_DISCRIMINATOR);
        assert_eq!(
            read_user_position(&short, 0).unwrap_err(),
            WickError::VenueAccountMismatch
        );
    }

    /// A borrow is not collateral. Counting it as one would report negative
    /// equity as positive backing.
    #[test]
    fn borrow_is_not_counted_as_collateral() {
        let mut data = synthetic_user(0, 0, 5_000_000_000);
        data[SPOT_POSITIONS_OFFSET + SPOT_BALANCE_TYPE_OFF] = 1; // Borrow
        let pos = read_user_position(&data, 0).unwrap();
        assert_eq!(pos.collateral, 0);
    }

    /// Offsets are pinned, not recomputed: this is the test that fails loudly
    /// if someone "tidies" the constants.
    #[test]
    fn user_layout_offsets_are_pinned() {
        assert_eq!(USER_SIZE, 4496);
        assert_eq!(PERP_POSITIONS_OFFSET, 424);
        assert_eq!(PERP_POSITION_STRIDE, 80);
        assert_eq!(PERP_POSITION_MARKET_INDEX_OFF_CHECK, 76);
        assert_eq!(SPOT_POSITIONS_OFFSET, 104);
        assert_eq!(SPOT_POSITION_STRIDE, 40);
        // The array must fit inside the account it is read from. In a `const`
        // block so this is a compile error rather than a test failure: a
        // position array that overruns the account is an out-of-bounds read on
        // chain, and that should never get as far as being run.
        const {
            assert!(
                PERP_POSITIONS_OFFSET + PERP_POSITION_COUNT * PERP_POSITION_STRIDE <= USER_SIZE
            );
            assert!(
                SPOT_POSITIONS_OFFSET + SPOT_POSITION_COUNT * SPOT_POSITION_STRIDE <= USER_SIZE
            );
        }
    }

    const PERP_POSITION_MARKET_INDEX_OFF_CHECK: usize = PERP_MARKET_INDEX_OFF;
}
