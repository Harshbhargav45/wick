//! Instruction dispatch and handlers (Phase 1).
//!
//! Account layout per instruction:
//!
//! * `InitGuard`    — [0] guard PDA (writable, created), [1] payer (signer,
//!   writable), [2] rent sysvar. Payload carries the policy + co_authority;
//!   the PDA itself is derived from `b"guard"` || venue_owner.
//! * `DepositMargin`— [0] guard (writable, program-owned), [1] owner (signer).
//!   Payload is the deposit `amount` (u128 LE). Credited to `collateral`.
//! * `WithdrawMargin`— [0] guard (writable, program-owned), [1] owner
//!   (signer), [2] co_authority (signer). Payload is `amount` (u128 LE).
//!   Enforces the §8.5 2-of-2 rule.
//! * `SetPaused`    — [0] route-config (writable, program-owned), [1] config
//!   authority (signer). Payload is `paused` (u8).

use pinocchio::error::ProgramResult;
use pinocchio::instruction::{cpi::Signer, seeds};
use pinocchio::{AccountView, Address};

use pinocchio_system::create_program_account_with_minimum_balance_signed;

use crate::account::{GuardState, GUARD_DATA_LEN};
use crate::error::WickError;
use crate::instruction::WickInstruction;
use crate::state::{ActionCaps, AuthorityRequirement, RouteConfig, VenuePolicy};

const GUARD_SEED: &[u8] = b"guard";

// -------------------------------------------------------------------------
// Payload parsing
// -------------------------------------------------------------------------

/// Parse the trailing `amount` (u128 LE) after the discriminator byte.
fn parse_amount(data: &[u8]) -> Result<u128, WickError> {
    let payload = data.get(1..).ok_or(WickError::InvalidInstruction)?;
    if payload.len() != 16 {
        return Err(WickError::InvalidInstruction);
    }
    Ok(u128::from_le_bytes(
        payload.try_into().map_err(|_| WickError::InvalidInstruction)?,
    ))
}

/// InitGuard payload layout (after the discriminator byte):
///   [0..32]   co_authority
///   [32]      authority_req (0 = Autonomous, 1 = CoSigned)
///   [33..49]  maintenance_bps (u128 LE)
///   [49..65]  trigger_buffer_bps (u128 LE)
///   [65..81]  fee_bps (u128 LE)
///   [81..97]  cap_top_up (u128 LE)
///   [97..113] cap_partial_close (u128 LE)
///   [113..129] cap_daily (u128 LE)
///   [129..145] take_profit (u128 LE; u128::MAX = none)
const INIT_PAYLOAD_LEN: usize = 145;

fn parse_policy(payload: &[u8]) -> Result<(VenuePolicy, [u8; 32]), WickError> {
    if payload.len() != INIT_PAYLOAD_LEN {
        return Err(WickError::InvalidInstruction);
    }
    let mut co_authority = [0u8; 32];
    co_authority.copy_from_slice(&payload[0..32]);

    let rd = |off: usize| -> Result<u128, WickError> {
        Ok(u128::from_le_bytes(
            payload[off..off + 16]
                .try_into()
                .map_err(|_| WickError::InvalidInstruction)?,
        ))
    };

    let authority_req = match payload[32] {
        0 => AuthorityRequirement::Autonomous,
        1 => AuthorityRequirement::CoSigned,
        _ => return Err(WickError::InvalidInstruction),
    };
    let take_profit = rd(129)?;
    let policy = VenuePolicy {
        maintenance_bps: rd(33)?,
        trigger_buffer_bps: rd(49)?,
        fee_bps: rd(65)?,
        authority: authority_req,
        caps: ActionCaps {
            top_up_usd_per_action: rd(81)?,
            partial_close_usd_per_action: rd(97)?,
            daily_total_usd: rd(113)?,
        },
        take_profit: if take_profit == u128::MAX { None } else { Some(take_profit) },
    };
    Ok((policy, co_authority))
}

// -------------------------------------------------------------------------
// Guard account load/store
// -------------------------------------------------------------------------

/// Read the program-owned guard account; rejects accounts we don't own or that
/// haven't been initialized with our layout.
fn load_guard(account: &AccountView, program_id: &Address) -> Result<GuardState, WickError> {
    if !account.owned_by(program_id) {
        return Err(WickError::WrongAccountOwner);
    }
    let data = account
        .try_borrow()
        .map_err(|_| WickError::NotInitialized)?;
    GuardState::from_bytes(&data).map_err(|_| WickError::NotInitialized)
}

/// Write a decoded guard state back into its account.
fn store_guard(account: &mut AccountView, state: &GuardState) -> Result<(), WickError> {
    let mut data = account
        .try_borrow_mut()
        .map_err(|_| WickError::NotInitialized)?;
    state
        .write_into(&mut data)
        .map_err(|_| WickError::NotInitialized)
}

// -------------------------------------------------------------------------
// Handlers
// -------------------------------------------------------------------------

fn init_guard(
    program_id: &Address,
    accounts: &mut [AccountView],
    data: &[u8],
) -> ProgramResult {
    let (guard, payer, rent) = split_3(accounts)?;
    if !payer.is_signer() {
        return Err(WickError::MissingOwnerAuthority.into());
    }
    let payload = data.get(1..).ok_or(WickError::InvalidInstruction)?;
    let (policy, co_authority) = parse_policy(payload)?;

    // The guard PDA is derived from `b"guard"` || venue_owner. We recover
    // venue_owner from the account address so an attacker can't forge the
    // stored owner. (Phase 1: the PDA is created here and its address equals
    // the account address; the seeds below let the runtime re-derive it.)
    let venue_owner_bytes = guard.address().to_bytes();
    let venue_owner: [u8; 32] = venue_owner_bytes;

    let bump = [0u8];
    let seeds = seeds!(GUARD_SEED, &venue_owner[..], &bump);
    let signer = Signer::from(&seeds);

    create_program_account_with_minimum_balance_signed(
        guard,
        GUARD_DATA_LEN,
        program_id,
        payer,
        Some(rent),
        &[signer],
    )?;

    let state = GuardState {
        venue: 0, // Phase 1: no venue adapter yet
        venue_owner,
        co_authority,
        authority_req: policy.authority,
        policy,
        collateral: 0,
        size: 0,
        entry: 0,
        current_price: 0,
        nonce: 0,
        last_check_slot: 0,
        pending: None,
    };
    store_guard(guard, &state)?;
    Ok(())
}

fn deposit_margin(
    program_id: &Address,
    accounts: &mut [AccountView],
    data: &[u8],
) -> ProgramResult {
    let (guard, owner) = split_2(accounts)?;
    if !owner.is_signer() {
        return Err(WickError::MissingOwnerAuthority.into());
    }
    let amount = parse_amount(data)?;

    let mut state = load_guard(guard, program_id)?;
    // Owner must match the guard's venue owner.
    if owner.address() != &Address::from(state.venue_owner) {
        return Err(WickError::SignerKeyMismatch.into());
    }
    state.collateral = state
        .collateral
        .checked_add(amount)
        .ok_or(WickError::MathOverflow)?;
    store_guard(guard, &state)?;
    Ok(())
}

fn withdraw_margin(
    program_id: &Address,
    accounts: &mut [AccountView],
    data: &[u8],
) -> ProgramResult {
    let (guard, owner, co_authority) = split_3(accounts)?;
    let amount = parse_amount(data)?;

    let state = load_guard(guard, program_id)?;
    // §8.5 — 2-of-2: owner (wallet) + co_authority must both sign and match.
    validate_withdraw(
        owner.is_signer(),
        owner.address(),
        &Address::from(state.venue_owner),
        co_authority.is_signer(),
        co_authority.address(),
        &Address::from(state.co_authority),
    )?;

    if amount > state.collateral {
        return Err(WickError::MathOverflow.into());
    }
    let mut new_state = state;
    new_state.collateral -= amount;
    store_guard(guard, &new_state)?;
    Ok(())
}

fn set_paused(
    program_id: &Address,
    accounts: &mut [AccountView],
    data: &[u8],
) -> ProgramResult {
    let (config, authority) = split_2(accounts)?;
    if !config.owned_by(program_id) {
        return Err(WickError::WrongAccountOwner.into());
    }
    if !authority.is_signer() {
        return Err(WickError::Unauthorized.into());
    }
    let paused = match data.get(1) {
        Some(1) => true,
        Some(0) => false,
        _ => return Err(WickError::InvalidInstruction.into()),
    };

    let mut cfg = RouteConfig::from_bytes(
        &config.try_borrow().map_err(|_| WickError::NotInitialized)?,
    )
    .map_err(|_| WickError::NotInitialized)?;
    if authority.address() != &Address::from(cfg.authority) {
        return Err(WickError::Unauthorized.into());
    }
    cfg.paused = paused;
    {
        let mut data = config
            .try_borrow_mut()
            .map_err(|_| WickError::NotInitialized)?;
        cfg.write_into(&mut data)
            .map_err(|_| WickError::NotInitialized)?;
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Account splitting helpers
// -------------------------------------------------------------------------

fn split_2(accounts: &mut [AccountView]) -> Result<(&mut AccountView, &mut AccountView), WickError> {
    if accounts.len() < 2 {
        return Err(WickError::InvalidInstruction);
    }
    let (first, rest) = accounts.split_at_mut(1);
    Ok((&mut first[0], &mut rest[0]))
}

fn split_3(
    accounts: &mut [AccountView],
) -> Result<(&mut AccountView, &mut AccountView, &mut AccountView), WickError> {
    if accounts.len() < 3 {
        return Err(WickError::InvalidInstruction);
    }
    let (first, rest) = accounts.split_at_mut(1);
    let (second, rest) = rest.split_at_mut(1);
    Ok((&mut first[0], &mut second[0], &mut rest[0]))
}

pub fn process_instruction(
    program_id: &Address,
    accounts: &mut [AccountView],
    data: &[u8],
) -> ProgramResult {
    let Some(discriminator_byte) = data.first().copied() else {
        return Err(WickError::InvalidInstruction.into());
    };
    let Some(ix) = WickInstruction::from_byte(discriminator_byte) else {
        return Err(WickError::InvalidInstruction.into());
    };

    match ix {
        WickInstruction::InitGuard => init_guard(program_id, accounts, data),
        WickInstruction::DepositMargin => deposit_margin(program_id, accounts, data),
        WickInstruction::WithdrawMargin => withdraw_margin(program_id, accounts, data),
        WickInstruction::SetPaused => set_paused(program_id, accounts, data),
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

    #[test]
    fn parse_amount_accepts_16_bytes() {
        let mut data = [0u8; 17];
        data[0] = 1;
        data[1..17].copy_from_slice(&42u128.to_le_bytes());
        assert_eq!(parse_amount(&data).unwrap(), 42);
    }

    #[test]
    fn parse_amount_rejects_short_payload() {
        assert_eq!(parse_amount(&[1, 2, 3]), Err(WickError::InvalidInstruction));
    }

    #[test]
    fn parse_policy_roundtrip() {
        let mut payload = [0u8; INIT_PAYLOAD_LEN];
        payload[32] = 1; // CoSigned
        payload[33..49].copy_from_slice(&500u128.to_le_bytes());
        payload[49..65].copy_from_slice(&500u128.to_le_bytes());
        payload[65..81].copy_from_slice(&10u128.to_le_bytes());
        payload[81..97].copy_from_slice(&1_000u128.to_le_bytes());
        payload[97..113].copy_from_slice(&2_000u128.to_le_bytes());
        payload[113..129].copy_from_slice(&5_000u128.to_le_bytes());
        payload[129..145].copy_from_slice(&60_000_000u128.to_le_bytes());
        payload[0..32].copy_from_slice(&[9u8; 32]);

        let (policy, co) = parse_policy(&payload).unwrap();
        assert_eq!(policy.authority, AuthorityRequirement::CoSigned);
        assert_eq!(policy.maintenance_bps, 500);
        assert_eq!(policy.caps.daily_total_usd, 5_000);
        assert_eq!(policy.take_profit, Some(60_000_000));
        assert_eq!(co, [9u8; 32]);
    }

    #[test]
    fn parse_policy_rejects_bad_authority() {
        let mut payload = [0u8; INIT_PAYLOAD_LEN];
        payload[32] = 7;
        assert_eq!(parse_policy(&payload).unwrap_err(), WickError::InvalidInstruction);
    }

    // ------------------------------------------------------------------
    //  End-to-end integration tests: construct real `AccountView` backing
    //  memory and drive `process_instruction`.
    // ------------------------------------------------------------------

    extern crate std;

    use crate::account::{GuardState, GUARD_DATA_LEN};
    use crate::state::{ActionCaps, AuthorityRequirement, RouteConfig, VenuePolicy};
    use crate::account::ROUTE_CONFIG_LEN;
    use pinocchio::account::{RuntimeAccount, NOT_BORROWED};
    use std::mem;
    use std::vec;
    use std::vec::Vec;

    const PROGRAM_ID: Address = Address::new_from_array([7u8; 32]);

    /// Owns a contiguous `RuntimeAccount` struct followed immediately by its
    /// data bytes, so `AccountView::new_unchecked` sees a valid layout.
    struct TestAccount {
        buf: Vec<u8>,
        view: AccountView,
    }

    impl TestAccount {
        fn new(
            address: Address,
            owner: Address,
            lamports: u64,
            data: &[u8],
            is_signer: bool,
            is_writable: bool,
        ) -> Self {
            let struct_size = size_of::<RuntimeAccount>();
            let mut buf = vec![0u8; struct_size + data.len()];
            let raw = buf.as_mut_ptr().cast::<RuntimeAccount>();
            // SAFETY: buf is exactly struct_size + data, aligned for the struct.
            unsafe {
                (*raw).borrow_state = NOT_BORROWED;
                (*raw).is_signer = is_signer as u8;
                (*raw).is_writable = is_writable as u8;
                (*raw).executable = 0;
                (*raw).padding = [0; 4];
                (*raw).address = address;
                (*raw).owner = owner;
                (*raw).lamports = lamports;
                (*raw).data_len = data.len() as u64;
                buf[struct_size..].copy_from_slice(data);
                let view = AccountView::new_unchecked(raw);
                TestAccount { buf, view }
            }
        }

        /// Return the account's live data bytes (the region immediately after
        /// the `RuntimeAccount` struct).
        fn data(&self) -> &[u8] {
            let struct_size = mem::size_of::<RuntimeAccount>();
            &self.buf[struct_size..]
        }
    }

    /// Build a sample initialized guard account matching `remove_owner` as the
    /// venue owner and `[5u8;32]` as co_authority.
    fn sample_guard(venue_owner: Address) -> Vec<u8> {
        let state = GuardState {
            venue: 0,
            venue_owner: venue_owner.to_bytes(),
            co_authority: [5u8; 32],
            authority_req: AuthorityRequirement::CoSigned,
            policy: VenuePolicy {
                maintenance_bps: 500,
                trigger_buffer_bps: 500,
                fee_bps: 10,
                authority: AuthorityRequirement::CoSigned,
                caps: ActionCaps {
                    top_up_usd_per_action: 1_000,
                    partial_close_usd_per_action: 2_000,
                    daily_total_usd: 5_000,
                },
                take_profit: Some(60_000_000),
            },
            collateral: 100_000_000,
            size: 100_000_000,
            entry: 50_000_000,
            current_price: 49_000_000,
            nonce: 0,
            last_check_slot: 0,
            pending: None,
        };
        let mut buf = vec![0u8; GUARD_DATA_LEN];
        state.write_into(&mut buf).unwrap();
        buf
    }

    #[test]
    fn deposit_increments_collateral() {
        let owner = addr(9);
        let guard_data = sample_guard(owner);
        let guard = TestAccount::new(
            PROGRAM_ID,        // address (unused for deposit)
            PROGRAM_ID,        // owned by program
            100,               // lamports
            &guard_data,
            false,             // not a signer (guard itself never signs)
            true,              // writable
        );
        let owner_acc = TestAccount::new(
            owner,
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,  // signer
            false,
        );

        let mut accounts = [guard.view, owner_acc.view];
        let data = [1u8, 0x2a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]; // Deposit 42

        let result = process_instruction(&PROGRAM_ID, &mut accounts, &data);
        assert!(result.is_ok());

        let new_state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(new_state.collateral, 100_000_042);
    }

    #[test]
    fn deposit_rejects_non_owner_signer() {
        let owner = addr(9);
        let guard_data = sample_guard(owner);
        let guard = TestAccount::new(
            PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true,
        );
        // Signer is a different key than the stored venue_owner.
        let stranger = TestAccount::new(
            addr(99), Address::new_from_array([0u8; 32]), 0, &[], true, false,
        );

        let mut accounts = [guard.view, stranger.view];
        let data = [1u8, 42, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let result = process_instruction(&PROGRAM_ID, &mut accounts, &data);
        assert_eq!(result, Err(WickError::SignerKeyMismatch.into()));

        // Collateral untouched.
        let new_state = GuardState::from_bytes(&guard_data).unwrap();
        assert_eq!(new_state.collateral, 100_000_000);
    }

    #[test]
    fn deposit_requires_signer_flag() {
        let owner = addr(9);
        let guard_data = sample_guard(owner);
        let guard = TestAccount::new(
            PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true,
        );
        // Correct key but is_signer = false.
        let owner_acc = TestAccount::new(
            owner, Address::new_from_array([0u8; 32]), 0, &[], false, false,
        );

        let mut accounts = [guard.view, owner_acc.view];
        let data = [1u8, 42, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let result = process_instruction(&PROGRAM_ID, &mut accounts, &data);
        assert_eq!(result, Err(WickError::MissingOwnerAuthority.into()));
    }

    #[test]
    fn withdraw_requires_both_sigs_end_to_end() {
        let owner = addr(9);
        let guard_data = sample_guard(owner);
        let guard = TestAccount::new(
            PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true,
        );
        let owner_acc = TestAccount::new(
            owner, Address::new_from_array([0u8; 32]), 0, &[], true, false,
        );
        // co_authority not signed (correct key, wrong flag).
        let co = TestAccount::new(
            addr(5), Address::new_from_array([0u8; 32]), 0, &[], false, false,
        );

        let mut accounts = [guard.view, owner_acc.view, co.view];
        let data = [2u8, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; // Withdraw 10
        let result = process_instruction(&PROGRAM_ID, &mut accounts, &data);
        assert_eq!(result, Err(WickError::MissingCoAuthority.into()));

        // Collateral untouched.
        let new_state = GuardState::from_bytes(&guard_data).unwrap();
        assert_eq!(new_state.collateral, 100_000_000);
    }

    #[test]
    fn withdraw_success_with_both_sigs() {
        let owner = addr(9);
        let guard_data = sample_guard(owner);
        let guard = TestAccount::new(
            PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true,
        );
        let owner_acc = TestAccount::new(
            owner, Address::new_from_array([0u8; 32]), 0, &[], true, false,
        );
        let co = TestAccount::new(
            addr(5), Address::new_from_array([0u8; 32]), 0, &[], true, false,
        );

        let mut accounts = [guard.view, owner_acc.view, co.view];
        let data = [2u8, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; // Withdraw 10
        let result = process_instruction(&PROGRAM_ID, &mut accounts, &data);
        assert!(result.is_ok());

        let new_state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(new_state.collateral, 100_000_000 - 10);
    }

    #[test]
    fn withdraw_rejects_over_balance() {
        let owner = addr(9);
        let guard_data = sample_guard(owner);
        let guard = TestAccount::new(
            PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true,
        );
        let owner_acc = TestAccount::new(
            owner, Address::new_from_array([0u8; 32]), 0, &[], true, false,
        );
        let co = TestAccount::new(
            addr(5), Address::new_from_array([0u8; 32]), 0, &[], true, false,
        );

        let mut accounts = [guard.view, owner_acc.view, co.view];
        // Withdraw u128::MAX — way over 100_000_000 collateral.
        let mut data = [2u8; 17];
        data[0] = 2;
        for b in data[1..].iter_mut() {
            *b = 0xff;
        }
        let result = process_instruction(&PROGRAM_ID, &mut accounts, &data);
        assert_eq!(result, Err(WickError::MathOverflow.into()));
    }

    #[test]
    fn withdraw_rejects_foreign_owner_account() {
        let owner = addr(9);
        let guard_data = sample_guard(owner);
        // Guard account owned by a DIFFERENT program.
        let guard = TestAccount::new(
            PROGRAM_ID,
            Address::new_from_array([0xee; 32]),
            100,
            &guard_data,
            false,
            true,
        );
        let owner_acc = TestAccount::new(
            owner, Address::new_from_array([0u8; 32]), 0, &[], true, false,
        );
        let co = TestAccount::new(
            addr(5), Address::new_from_array([0u8; 32]), 0, &[], true, false,
        );

        let mut accounts = [guard.view, owner_acc.view, co.view];
        let data = [2u8, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let result = process_instruction(&PROGRAM_ID, &mut accounts, &data);
        assert_eq!(result, Err(WickError::WrongAccountOwner.into()));
    }

    #[test]
    fn set_paused_flips_flag() {
        let cfg = RouteConfig {
            authority: [3u8; 32],
            paused: false,
            _padding: [0u8; 31],
        };
        let mut buf = vec![0u8; ROUTE_CONFIG_LEN];
        cfg.write_into(&mut buf).unwrap();

        let config_acc = TestAccount::new(
            PROGRAM_ID, PROGRAM_ID, 100, &buf, false, true,
        );
        let authority = TestAccount::new(
            addr(3), Address::new_from_array([0u8; 32]), 0, &[], true, false,
        );

        let mut accounts = [config_acc.view, authority.view];
        let data = [3u8, 1]; // SetPaused true
        let result = process_instruction(&PROGRAM_ID, &mut accounts, &data);
        assert!(result.is_ok());

        let new_cfg = RouteConfig::from_bytes(config_acc.data()).unwrap();
        assert!(new_cfg.paused);
    }

    #[test]
    fn set_paused_rejects_wrong_authority() {
        let cfg = RouteConfig {
            authority: [3u8; 32],
            paused: false,
            _padding: [0u8; 31],
        };
        let mut buf = vec![0u8; ROUTE_CONFIG_LEN];
        cfg.write_into(&mut buf).unwrap();

        let config_acc = TestAccount::new(
            PROGRAM_ID, PROGRAM_ID, 100, &buf, false, true,
        );
        // Signer does not match stored authority [3u8;32].
        let wrong = TestAccount::new(
            addr(4), Address::new_from_array([0u8; 32]), 0, &[], true, false,
        );

        let mut accounts = [config_acc.view, wrong.view];
        let data = [3u8, 1];
        let result = process_instruction(&PROGRAM_ID, &mut accounts, &data);
        assert_eq!(result, Err(WickError::Unauthorized.into()));
    }
}