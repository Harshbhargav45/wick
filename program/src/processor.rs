//! Instruction dispatch and handlers (Phase 1).
//!
//! Account layout per instruction:
//!
//! * `InitGuard`    — [0] guard PDA (writable, created), [1] owner (signer),
//!   [2] payer (signer, writable), [3] rent sysvar. Payload = [bump | policy
//!   blob]; the PDA is derived from `b"guard"` || owner || bump.
//! * `DepositMargin`— [0] guard (writable, program-owned), [1] owner (signer).
//!   Payload is the deposit `amount` (u128 LE). Credited to `collateral`.
//! * `WithdrawMargin`— [0] guard (writable, program-owned), [1] owner
//!   (signer), [2] co_authority (signer). Payload is `amount` (u128 LE).
//!   Enforces the §8.5 2-of-2 rule.
//! * `SetPaused`    — [0] route-config (writable, program-owned), [1] config
//!   authority (signer). Payload is `paused` (u8).

use pinocchio::error::ProgramResult;
use pinocchio::instruction::{cpi::Signer, seeds};
use pinocchio::sysvars::clock::Clock;
use pinocchio::{AccountView, Address};

use pinocchio_system::instructions::CreateAccount;

use crate::account::{GuardState, PendingIx, GUARD_DATA_LEN};
use crate::delegation;
use crate::drift::{DriftPlaceOrderAccounts, ReduceDirection, ReduceOrderParams, VENUE_DRIFT};
use crate::error::WickError;
use crate::flash::{ClosePositionAccounts, ClosePositionParams, VENUE_FLASH};
use crate::instruction::WickInstruction;
use crate::jupiter::{build_tp_safety_net, VENUE_JUPITER};
use crate::state::{
    accept_tick, guard_act, select_action, track_tick_freshness, Action, ActionCaps,
    AuthorityRequirement, DispatchRegime, HealthSnapshot, RouteConfig, VenuePolicy,
};

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
        payload
            .try_into()
            .map_err(|_| WickError::InvalidInstruction)?,
    ))
}

/// InitGuard payload layout (after the discriminator byte):
///   [0]       venue (u8 — which venue adapter owns the position; 0 = none)
///   [1..33]   co_authority
///   [33]      authority_req (0 = Autonomous, 1 = CoSigned)
///   [34..50]  maintenance_bps (u128 LE)
///   [50..66]  trigger_buffer_bps (u128 LE)
///   [66..82]  fee_bps (u128 LE)
///   [82..98]  cap_top_up (u128 LE)
///   [98..114] cap_partial_close (u128 LE)
///   [114..130] cap_daily (u128 LE)
///   [130..146] take_profit (u128 LE; u128::MAX = none)
///   [146..148] drift_market_index (u16 LE; venue = VENUE_DRIFT)
///   [148..150] drift_subaccount_id (u16 LE; venue = VENUE_DRIFT)
const INIT_PAYLOAD_LEN: usize = 150;

fn parse_policy(payload: &[u8]) -> Result<(VenuePolicy, [u8; 32], u8, u16, u16), WickError> {
    if payload.len() != INIT_PAYLOAD_LEN {
        return Err(WickError::InvalidInstruction);
    }
    let mut co_authority = [0u8; 32];
    co_authority.copy_from_slice(&payload[1..33]);

    let rd = |off: usize| -> Result<u128, WickError> {
        Ok(u128::from_le_bytes(
            payload[off..off + 16]
                .try_into()
                .map_err(|_| WickError::InvalidInstruction)?,
        ))
    };

    let authority_req = match payload[33] {
        0 => AuthorityRequirement::Autonomous,
        1 => AuthorityRequirement::CoSigned,
        _ => return Err(WickError::InvalidInstruction),
    };
    let take_profit = rd(130)?;
    let policy = VenuePolicy {
        maintenance_bps: rd(34)?,
        trigger_buffer_bps: rd(50)?,
        fee_bps: rd(66)?,
        authority: authority_req,
        caps: ActionCaps {
            top_up_usd_per_action: rd(82)?,
            partial_close_usd_per_action: rd(98)?,
            daily_total_usd: rd(114)?,
        },
        take_profit: if take_profit == u128::MAX {
            None
        } else {
            Some(take_profit)
        },
    };
    let drift_market_index = u16::from_le_bytes(
        payload[146..148]
            .try_into()
            .map_err(|_| WickError::InvalidInstruction)?,
    );
    let drift_subaccount_id = u16::from_le_bytes(
        payload[148..150]
            .try_into()
            .map_err(|_| WickError::InvalidInstruction)?,
    );
    Ok((
        policy,
        co_authority,
        payload[0],
        drift_market_index,
        drift_subaccount_id,
    ))
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
fn store_guard(account: &AccountView, state: &GuardState) -> Result<(), WickError> {
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

fn init_guard(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let (guard, owner, payer, rent) = split_4(accounts)?;
    if !owner.is_signer() || !payer.is_signer() {
        return Err(WickError::MissingOwnerAuthority.into());
    }

    // Init payload: [0] bump (u8) then the policy blob.
    let bump = *data.get(1).ok_or(WickError::InvalidInstruction)?;
    let payload = data.get(2..).ok_or(WickError::InvalidInstruction)?;
    let (policy, co_authority, venue, drift_market_index, drift_subaccount_id) =
        parse_policy(payload)?;

    // The guard PDA is derived from `b"guard" || owner_pubkey || bump`. The
    // owner pubkey is supplied as a signed account, so an attacker cannot
    // forge the stored owner. The runtime re-derives the address from these
    // seeds during the CPI and refuses creation if it does not match
    // `guard.address()`.
    let venue_owner = owner.address().to_bytes();
    let bump = [bump];
    let seeds = seeds!(GUARD_SEED, &venue_owner[..], &bump);
    let signer = Signer::from(&seeds);

    if guard.lamports() == 0 {
        let create_account = CreateAccount::with_minimum_balance(
            payer,
            guard,
            GUARD_DATA_LEN as u64,
            program_id,
            Some(rent),
        )?;
        create_account.invoke_signed(&[signer])?;
    }

    let state = GuardState {
        venue,
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
        pending_ix: None,
        degraded: false,
        stale_streak: 0,
        drift_market_index,
        drift_subaccount_id,
    };
    store_guard(guard, &state)?;
    Ok(())
}

fn deposit_margin(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
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

fn withdraw_margin(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
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

fn set_paused(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
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

    let mut cfg =
        RouteConfig::from_bytes(&config.try_borrow().map_err(|_| WickError::NotInitialized)?)
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

/// Record the watched position's snapshot. Only the guard owner (venue owner)
/// may set it — this is the enrollment step after the position is opened.
///
/// Account layout: [0] guard (writable, program-owned), [1] owner (signer).
/// Data (after discriminator): collateral (u128), size (i128), entry (u128).
fn update_position(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let (guard, owner) = split_2(accounts)?;
    if !owner.is_signer() {
        return Err(WickError::MissingOwnerAuthority.into());
    }
    let payload = data.get(1..).ok_or(WickError::InvalidInstruction)?;
    if payload.len() != 48 {
        return Err(WickError::InvalidInstruction.into());
    }
    let collateral = u128::from_le_bytes(payload[0..16].try_into().unwrap());
    let size = i128::from_le_bytes(payload[16..32].try_into().unwrap());
    let entry = u128::from_le_bytes(payload[32..48].try_into().unwrap());

    let mut state = load_guard(guard, program_id)?;
    if owner.address() != &Address::from(state.venue_owner) {
        return Err(WickError::SignerKeyMismatch.into());
    }
    state.collateral = collateral;
    state.size = size;
    state.entry = entry;
    store_guard(guard, &state)?;
    Ok(())
}

/// Owner confirms the pending owner-signed venue instruction, committing the
/// expected nonce this tick must use (§8.4 CoSigned / §8.7 Jupiter).
///
/// The guard builds the owner-signed safety-net instruction on `OnPriceTick`
/// but holds it pending and does **not** advance the nonce. The owner lands
/// that instruction on L1 with their own signature; `Confirm` records that it
/// happened: it commits `expected_nonce` as the new base and clears the pending
/// instruction so a later genuine breach is not mistaken for a replay.
///
/// Account layout: [0] guard (writable, program-owned), [1] owner (signer).
/// Data (after discriminator): none — the pending instruction is on the guard.
fn confirm_pending(program_id: &Address, accounts: &[AccountView], _data: &[u8]) -> ProgramResult {
    let (guard, owner) = split_2(accounts)?;
    if !guard.owned_by(program_id) {
        return Err(WickError::WrongAccountOwner.into());
    }
    if !owner.is_signer() {
        return Err(WickError::MissingOwnerAuthority.into());
    }
    let mut state = load_guard(guard, program_id)?;
    if owner.address() != &Address::from(state.venue_owner) {
        return Err(WickError::SignerKeyMismatch.into());
    }
    let px = state.pending_ix.ok_or(WickError::NoPendingConfirm)?;
    // §8.4 — the nonce commits only when the owner confirms on L1.
    state.nonce = px.expected_nonce;
    state.pending_ix = None;
    state.pending = None;
    store_guard(guard, &state)?;
    Ok(())
}

// -------------------------------------------------------------------------
// §7.2 critical path — price tick
// -------------------------------------------------------------------------

/// OnPriceTick data layout (after the discriminator byte):
///   [0..16]  price (u128 LE, 6-decimal scale)
///   [16..24] tick nonce (u64 LE — monotonic, supplied by the tick source)
///   [24]     guard PDA bump (needed to sign the autonomous venue CPI)
///
/// Account layout:
///   [0]    guard (writable, program-owned)
///   [1]    clock sysvar (readonly)
///   [2..]  venue adapter accounts (only consumed for an autonomous Flash close)
fn on_price_tick(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let (guard, clock) = split_2(accounts)?;
    if !guard.owned_by(program_id) {
        return Err(WickError::WrongAccountOwner.into());
    }
    let current_slot = Clock::from_account_view(clock)
        .map_err(|_| WickError::InvalidInstruction)?
        .slot;

    let payload = data.get(1..).ok_or(WickError::InvalidInstruction)?;
    if payload.len() < 25 {
        return Err(WickError::InvalidInstruction.into());
    }
    let price = u128::from_le_bytes(payload[0..16].try_into().unwrap());
    let tick_nonce = u64::from_le_bytes(payload[16..24].try_into().unwrap());
    let bump = payload[24];

    let mut state = load_guard(guard, program_id)?;

    // §8.1.3 — reject stale ticks, tracking the degraded streak. A rejected
    // tick is not "do nothing this tick": it updates the streak/flag so the
    // guard can never silently protect against dead data.
    let fresh = accept_tick(current_slot, state.last_check_slot);
    let (streak, degraded) = track_tick_freshness(state.stale_streak, fresh);
    state.stale_streak = streak;
    state.degraded = degraded;
    state.current_price = price;
    state.last_check_slot = current_slot;

    if !fresh {
        store_guard(guard, &state)?;
        return Ok(());
    }

    // §8.2 — health → nonce → caps → action selection (§7.2 ordering).
    let health = HealthSnapshot {
        collateral: state.collateral,
        size: state.size,
        entry: state.entry,
        current_price: price,
    };
    let Some(action) = select_action(&health, &state.policy, tick_nonce, state.nonce)? else {
        store_guard(guard, &state)?; // healthy — snapshot updated, nothing else
        return Ok(());
    };

    // §8.4 — two-regime authority dispatch.
    let (regime, expected_nonce) = guard_act(&state.policy, state.nonce)?;
    match regime {
        DispatchRegime::Autonomous => {
            match execute_autonomous(&state, action, accounts, bump) {
                Ok(VenueOutcome::Executed) => {
                    // Nonce commits only when the venue action actually lands.
                    state.nonce = tick_nonce;
                }
                Ok(VenueOutcome::Escalate) | Err(WickError::UnsupportedVenueAction) => {
                    state.pending = Some(Action::EscalateManualReview);
                }
                Err(e) => return Err(e.into()),
            }
        }
        DispatchRegime::CoSigned => {
            // §8.4 / §8.7 — Jupiter is build-only for the guard. Build the
            // owner-signed safety-net instruction and hold it as pending; the
            // nonce must NOT advance until the owner co-signs on L1. The guard
            // never signs or submits it (no fake autonomy).
            if state.venue == VENUE_JUPITER {
                if let Action::TakeProfit = action {
                    let now = Clock::from_account_view(clock)
                        .map_err(|_| WickError::InvalidInstruction)?
                        .unix_timestamp;
                    let data = build_tp_safety_net(
                        state.policy.take_profit.unwrap_or(state.current_price),
                        now,
                    )?;
                    state.pending_ix = Some(PendingIx {
                        expected_nonce,
                        data,
                    });
                }
            }
            state.pending = Some(action);
        }
    }
    store_guard(guard, &state)?;
    Ok(())
}

/// Outcome of an autonomous venue execution.
enum VenueOutcome {
    /// The venue action landed on-chain; the nonce may commit.
    Executed,
    /// No venue action is possible; surface manual review.
    Escalate,
}

/// Execute an action through the venue adapter in the autonomous tier. The
/// guard PDA signs as the position owner — that is what ER delegation (§8.6)
/// unlocks for sub-50ms execution.
fn execute_autonomous(
    state: &GuardState,
    action: Action,
    accounts: &[AccountView],
    bump: u8,
) -> Result<VenueOutcome, WickError> {
    if state.venue == VENUE_JUPITER {
        // §8.7 — Jupiter is the co-signed tier. The guard cannot fake the
        // position owner's signature, so it never executes Jupiter
        // instructions autonomously. The safety-net TP/SL instruction data is
        // the owner's to sign (§8.4 CoSigned); autonomy here is not possible.
        return Err(WickError::UnsupportedVenueAction);
    }
    if state.venue == VENUE_DRIFT {
        return execute_drift_autonomous(state, action, accounts, bump);
    }
    if state.venue != VENUE_FLASH {
        return Err(WickError::UnsupportedVenueAction);
    }
    match action {
        Action::PartialClose { .. } | Action::TakeProfit => {
            // Flash's close_position closes the whole position, so both the
            // guard's partial-close and take-profit resolve to a protective
            // full close here. Conservative by design: exiting beats being
            // liquidated.
            let venue_accounts = accounts.get(2..).ok_or(WickError::InvalidInstruction)?;
            let adapter = ClosePositionAccounts::from_account_views(venue_accounts)?;
            let exit_price =
                u64::try_from(state.current_price).map_err(|_| WickError::MathOverflow)?;
            let params = ClosePositionParams { price: exit_price };

            let bump_bytes = [bump];
            let seeds = seeds!(GUARD_SEED, &state.venue_owner[..], &bump_bytes);
            adapter
                .invoke(&params, &[Signer::from(&seeds)])
                .map_err(|_| WickError::VenueCpi)?;
            Ok(VenueOutcome::Executed)
        }
        Action::TopUp { .. } => {
            // No Flash collateral-add CPI in the adapter yet. Escalate rather
            // than error so the tick's state progress (slot/nonce/snapshot)
            // still persists.
            Err(WickError::UnsupportedVenueAction)
        }
        Action::EscalateManualReview => Ok(VenueOutcome::Escalate),
    }
}

/// Execute a Drift perp reduce in the autonomous tier.
///
/// The guard signs `place_perp_order` as the venue owner's *delegate* on the
/// configured Drift sub-account (stored at init). Drift guarantees delegates
/// cannot withdraw funds, but it does not scope order placement on its side —
/// `reduce_only` is therefore forced by the adapter's serialization, and the
/// perp market being reduced is the one pinned in guard state at init.
fn execute_drift_autonomous(
    state: &GuardState,
    action: Action,
    accounts: &[AccountView],
    bump: u8,
) -> Result<VenueOutcome, WickError> {
    // Reduce fraction: a TakeProfit closes the watched size in full; a Partial
    // only shrinks the position by `fraction_bps` (already capped in §8.2).
    let fraction_bps: u128 = match action {
        Action::PartialClose { fraction_bps } => fraction_bps,
        Action::TakeProfit => 10_000,
        Action::TopUp { .. } | Action::EscalateManualReview => {
            return Err(WickError::UnsupportedVenueAction)
        }
    };

    let venue_accounts = accounts.get(2..).ok_or(WickError::InvalidInstruction)?;
    let adapter = DriftPlaceOrderAccounts::from_account_views(venue_accounts)?;

    // Reduce against the current position: a long reduces by selling (Drift
    // PositionDirection::Short), a short by buying (PositionDirection::Long).
    // `place_perp_order` signs as the `delegate` the venue owner set for
    // `state.drift_subaccount_id`, so the guard never adds exposure and the
    // `reduce_only` flag is forced by `ReduceOrderParams`'s serializer.
    let direction = if state.size >= 0 {
        ReduceDirection::Short
    } else {
        ReduceDirection::Long
    };
    // fraction of watched magnitude, carry in u128 then narrow.
    let abs_size = state.size.unsigned_abs();
    let reduce = abs_size.saturating_mul(fraction_bps) / 10_000;
    let reduce_size = u64::try_from(reduce).map_err(|_| WickError::MathOverflow)?;
    if reduce_size == 0 {
        return Ok(VenueOutcome::Escalate);
    }
    let price = u64::try_from(state.current_price).map_err(|_| WickError::MathOverflow)?;
    let params = ReduceOrderParams {
        market_index: state.drift_market_index,
        direction,
        base_asset_amount: reduce_size,
        price,
    };

    let bump_bytes = [bump];
    let seeds = seeds!(GUARD_SEED, &state.venue_owner[..], &bump_bytes);
    adapter
        .invoke(&params, &[Signer::from(&seeds)])
        .map_err(|_| WickError::VenueCpi)?;
    Ok(VenueOutcome::Executed)
}

// -------------------------------------------------------------------------
// Account splitting helpers
// -------------------------------------------------------------------------

fn split_2(accounts: &[AccountView]) -> Result<(&AccountView, &AccountView), WickError> {
    if accounts.len() < 2 {
        return Err(WickError::InvalidInstruction);
    }
    let (first, rest) = accounts.split_at(1);
    Ok((&first[0], &rest[0]))
}

fn split_3(
    accounts: &[AccountView],
) -> Result<(&AccountView, &AccountView, &AccountView), WickError> {
    if accounts.len() < 3 {
        return Err(WickError::InvalidInstruction);
    }
    let (first, rest) = accounts.split_at(1);
    let (second, rest) = rest.split_at(1);
    Ok((&first[0], &second[0], &rest[0]))
}

fn split_4(
    accounts: &[AccountView],
) -> Result<(&AccountView, &AccountView, &AccountView, &AccountView), WickError> {
    if accounts.len() < 4 {
        return Err(WickError::InvalidInstruction);
    }
    let (first, rest) = accounts.split_at(1);
    let (second, rest) = rest.split_at(1);
    let (third, rest) = rest.split_at(1);
    Ok((&first[0], &second[0], &third[0], &rest[0]))
}

pub fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let Some(discriminator_byte) = data.first().copied() else {
        return Err(WickError::InvalidInstruction.into());
    };

    // The Delegation Program's undelegation callback uses a fixed 8-byte
    // discriminator, distinct from the 1-byte `WickInstruction` space.
    if data.len() >= 8
        && data[..8] == ephemeral_rollups_pinocchio::consts::EXTERNAL_UNDELEGATE_DISCRIMINATOR
    {
        return crate::delegation::process_undelegation_callback(program_id, accounts, &data[8..]);
    }

    let Some(ix) = WickInstruction::from_byte(discriminator_byte) else {
        return Err(WickError::InvalidInstruction.into());
    };

    match ix {
        WickInstruction::InitGuard => init_guard(program_id, accounts, data),
        WickInstruction::DepositMargin => deposit_margin(program_id, accounts, data),
        WickInstruction::WithdrawMargin => withdraw_margin(program_id, accounts, data),
        WickInstruction::SetPaused => set_paused(program_id, accounts, data),
        WickInstruction::Delegate => delegation::process_delegate(accounts, &data[1..]),
        WickInstruction::CommitAndUndelegate => delegation::process_commit_and_undelegate(accounts),
        WickInstruction::Commit => delegation::process_commit(accounts),
        WickInstruction::OnPriceTick => on_price_tick(program_id, accounts, data),
        WickInstruction::UpdatePosition => update_position(program_id, accounts, data),
        WickInstruction::ConfirmYes => confirm_pending(program_id, accounts, data),
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
        payload[0] = 1; // venue = Flash
        payload[1..33].copy_from_slice(&[9u8; 32]); // co_authority
        payload[33] = 1; // CoSigned
        payload[34..50].copy_from_slice(&500u128.to_le_bytes());
        payload[50..66].copy_from_slice(&500u128.to_le_bytes());
        payload[66..82].copy_from_slice(&10u128.to_le_bytes());
        payload[82..98].copy_from_slice(&1_000u128.to_le_bytes());
        payload[98..114].copy_from_slice(&2_000u128.to_le_bytes());
        payload[114..130].copy_from_slice(&5_000u128.to_le_bytes());
        payload[130..146].copy_from_slice(&60_000_000u128.to_le_bytes());
        payload[146..148].copy_from_slice(&9u16.to_le_bytes());
        payload[148..150].copy_from_slice(&3u16.to_le_bytes());

        let (policy, co, venue, market_index, subaccount_id) = parse_policy(&payload).unwrap();
        assert_eq!(venue, 1);
        assert_eq!(policy.authority, AuthorityRequirement::CoSigned);
        assert_eq!(policy.maintenance_bps, 500);
        assert_eq!(policy.caps.daily_total_usd, 5_000);
        assert_eq!(policy.take_profit, Some(60_000_000));
        assert_eq!(co, [9u8; 32]);
        assert_eq!(market_index, 9);
        assert_eq!(subaccount_id, 3);
    }

    #[test]
    fn parse_policy_rejects_bad_authority() {
        let mut payload = [0u8; INIT_PAYLOAD_LEN];
        payload[33] = 7;
        assert_eq!(
            parse_policy(&payload).unwrap_err(),
            WickError::InvalidInstruction
        );
    }

    // ------------------------------------------------------------------
    //  End-to-end integration tests: construct real `AccountView` backing
    //  memory and drive `process_instruction`.
    // ------------------------------------------------------------------

    extern crate std;

    use crate::account::{GuardState, GUARD_DATA_LEN, PENDING_IX_DATA_LEN, ROUTE_CONFIG_LEN};
    use crate::state::{Action, ActionCaps, AuthorityRequirement, RouteConfig, VenuePolicy};
    use pinocchio::account::{RuntimeAccount, NOT_BORROWED};
    use pinocchio::sysvars::clock::CLOCK_ID;
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
                (*raw).resize_delta = 0;
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
            pending_ix: None,
            degraded: false,
            stale_streak: 0,
            drift_market_index: 0,
            drift_subaccount_id: 0,
        };
        let mut buf = vec![0u8; GUARD_DATA_LEN];
        state.write_into(&mut buf).unwrap();
        buf
    }

    /// Build a guard with generous caps so `select_action` resolves to a
    /// `TopUp` on a breach (keeps tick assertions deterministic).
    fn make_guard(
        authority: AuthorityRequirement,
        venue: u8,
        nonce: u64,
        last_check_slot: u64,
    ) -> Vec<u8> {
        let state = GuardState {
            venue,
            venue_owner: [9u8; 32],
            co_authority: [5u8; 32],
            authority_req: authority,
            policy: VenuePolicy {
                maintenance_bps: 500,
                trigger_buffer_bps: 500,
                fee_bps: 10,
                authority,
                caps: ActionCaps {
                    top_up_usd_per_action: u128::MAX,
                    partial_close_usd_per_action: u128::MAX,
                    daily_total_usd: u128::MAX,
                },
                take_profit: None,
            },
            collateral: 100_000_000,
            size: 100_000_000,
            entry: 50_000_000,
            current_price: 50_000_000,
            nonce,
            last_check_slot,
            pending: None,
            pending_ix: None,
            degraded: false,
            stale_streak: 0,
            drift_market_index: 0,
            drift_subaccount_id: 0,
        };
        let mut buf = vec![0u8; GUARD_DATA_LEN];
        state.write_into(&mut buf).unwrap();
        buf
    }

    /// OnPriceTick payload: [7, price:16, nonce:8, bump:1].
    fn tick_data(price: u128, nonce: u64, bump: u8) -> [u8; 26] {
        let mut d = [0u8; 26];
        d[0] = 7;
        d[1..17].copy_from_slice(&price.to_le_bytes());
        d[17..25].copy_from_slice(&nonce.to_le_bytes());
        d[25] = bump;
        d
    }

    /// A Clock sysvar account reporting `slot`. Address must be CLOCK_ID or
    /// `Clock::from_account_view` rejects it.
    fn clock_account(slot: u64) -> TestAccount {
        let mut data = vec![0u8; 40];
        data[0..8].copy_from_slice(&slot.to_le_bytes());
        TestAccount::new(
            CLOCK_ID,
            Address::new_from_array([0u8; 32]),
            0,
            &data,
            false,
            false,
        )
    }

    #[test]
    fn deposit_increments_collateral() {
        let owner = addr(9);
        let guard_data = sample_guard(owner);
        let guard = TestAccount::new(
            PROGRAM_ID, // address (unused for deposit)
            PROGRAM_ID, // owned by program
            100,        // lamports
            &guard_data,
            false, // not a signer (guard itself never signs)
            true,  // writable
        );
        let owner_acc = TestAccount::new(
            owner,
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true, // signer
            false,
        );

        let accounts = [guard.view, owner_acc.view];
        let data = [
            1u8, 0x2a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00,
        ]; // Deposit 42

        let result = process_instruction(&PROGRAM_ID, &accounts, &data);
        assert!(result.is_ok());

        let new_state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(new_state.collateral, 100_000_042);
    }

    #[test]
    fn deposit_rejects_non_owner_signer() {
        let owner = addr(9);
        let guard_data = sample_guard(owner);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        // Signer is a different key than the stored venue_owner.
        let stranger = TestAccount::new(
            addr(99),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );

        let accounts = [guard.view, stranger.view];
        let data = [1u8, 42, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let result = process_instruction(&PROGRAM_ID, &accounts, &data);
        assert_eq!(result, Err(WickError::SignerKeyMismatch.into()));

        // Collateral untouched.
        let new_state = GuardState::from_bytes(&guard_data).unwrap();
        assert_eq!(new_state.collateral, 100_000_000);
    }

    #[test]
    fn deposit_requires_signer_flag() {
        let owner = addr(9);
        let guard_data = sample_guard(owner);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        // Correct key but is_signer = false.
        let owner_acc = TestAccount::new(
            owner,
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            false,
            false,
        );

        let accounts = [guard.view, owner_acc.view];
        let data = [1u8, 42, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let result = process_instruction(&PROGRAM_ID, &accounts, &data);
        assert_eq!(result, Err(WickError::MissingOwnerAuthority.into()));
    }

    #[test]
    fn withdraw_requires_both_sigs_end_to_end() {
        let owner = addr(9);
        let guard_data = sample_guard(owner);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        let owner_acc = TestAccount::new(
            owner,
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );
        // co_authority not signed (correct key, wrong flag).
        let co = TestAccount::new(
            addr(5),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            false,
            false,
        );

        let accounts = [guard.view, owner_acc.view, co.view];
        let data = [2u8, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; // Withdraw 10
        let result = process_instruction(&PROGRAM_ID, &accounts, &data);
        assert_eq!(result, Err(WickError::MissingCoAuthority.into()));

        // Collateral untouched.
        let new_state = GuardState::from_bytes(&guard_data).unwrap();
        assert_eq!(new_state.collateral, 100_000_000);
    }

    #[test]
    fn withdraw_success_with_both_sigs() {
        let owner = addr(9);
        let guard_data = sample_guard(owner);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        let owner_acc = TestAccount::new(
            owner,
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );
        let co = TestAccount::new(
            addr(5),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );

        let accounts = [guard.view, owner_acc.view, co.view];
        let data = [2u8, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; // Withdraw 10
        let result = process_instruction(&PROGRAM_ID, &accounts, &data);
        assert!(result.is_ok());

        let new_state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(new_state.collateral, 100_000_000 - 10);
    }

    #[test]
    fn withdraw_rejects_over_balance() {
        let owner = addr(9);
        let guard_data = sample_guard(owner);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        let owner_acc = TestAccount::new(
            owner,
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );
        let co = TestAccount::new(
            addr(5),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );

        let accounts = [guard.view, owner_acc.view, co.view];
        // Withdraw u128::MAX — way over 100_000_000 collateral.
        let mut data = [2u8; 17];
        data[0] = 2;
        for b in data[1..].iter_mut() {
            *b = 0xff;
        }
        let result = process_instruction(&PROGRAM_ID, &accounts, &data);
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
            owner,
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );
        let co = TestAccount::new(
            addr(5),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );

        let accounts = [guard.view, owner_acc.view, co.view];
        let data = [2u8, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let result = process_instruction(&PROGRAM_ID, &accounts, &data);
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

        let config_acc = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &buf, false, true);
        let authority = TestAccount::new(
            addr(3),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );

        let accounts = [config_acc.view, authority.view];
        let data = [3u8, 1]; // SetPaused true
        let result = process_instruction(&PROGRAM_ID, &accounts, &data);
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

        let config_acc = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &buf, false, true);
        // Signer does not match stored authority [3u8;32].
        let wrong = TestAccount::new(
            addr(4),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );

        let accounts = [config_acc.view, wrong.view];
        let data = [3u8, 1];
        let result = process_instruction(&PROGRAM_ID, &accounts, &data);
        assert_eq!(result, Err(WickError::Unauthorized.into()));
    }

    #[test]
    fn delegate_requires_full_account_set() {
        // A `Delegate` (discriminator 4) needs 8 accounts + optional validator.
        // With too few accounts the SDK's `delegate_account` returns
        // `NotEnoughAccountKeys` before doing any CPI work.
        let empty: [AccountView; 0] = [];
        let data = [4u8, 0]; // bump = 0
        let result = delegation::process_delegate(&empty, &data[1..]);
        assert!(result.is_err());
    }

    #[test]
    fn commit_and_undelegate_requires_signer() {
        // `CommitAndUndelegate` (discriminator 5) requires a signed payer.
        // A non-signer payer must be rejected before any magic-program CPI.
        let payer_acc = TestAccount::new(
            addr(1),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            false,
            false,
        );
        let guard_acc = TestAccount::new(
            PROGRAM_ID,
            PROGRAM_ID,
            100,
            &[0u8; GUARD_DATA_LEN],
            false,
            true,
        );
        let magic = TestAccount::new(
            addr(2),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            false,
            false,
        );
        let ctx = TestAccount::new(
            addr(3),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            false,
            false,
        );
        let accounts = [payer_acc.view, guard_acc.view, magic.view, ctx.view];
        let result = delegation::process_commit_and_undelegate(&accounts);
        assert_eq!(result, Err(WickError::MissingOwnerAuthority.into()));
    }

    #[test]
    fn commit_requires_signer() {
        let payer_acc = TestAccount::new(
            addr(1),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            false,
            false,
        );
        let guard_acc = TestAccount::new(
            PROGRAM_ID,
            PROGRAM_ID,
            100,
            &[0u8; GUARD_DATA_LEN],
            false,
            true,
        );
        let magic = TestAccount::new(
            addr(2),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            false,
            false,
        );
        let ctx = TestAccount::new(
            addr(3),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            false,
            false,
        );
        let accounts = [payer_acc.view, guard_acc.view, magic.view, ctx.view];
        let result = delegation::process_commit(&accounts);
        assert_eq!(result, Err(WickError::MissingOwnerAuthority.into()));
    }

    #[test]
    fn delegate_unknown_byte_still_unhandled() {
        // Discriminator 9 is not assigned; must be rejected.
        let empty: [AccountView; 0] = [];
        let data = [9u8, 0];
        let result = process_instruction(&PROGRAM_ID, &empty, &data);
        assert_eq!(result, Err(WickError::InvalidInstruction.into()));
    }

    // ------------------------------------------------------------------
    //  OnPriceTick — §7.2 critical path
    // ------------------------------------------------------------------

    /// Drive one tick against a guard. `guard_acc` is the writable guard
    /// account; a fresh clock account is created per call.
    fn run_tick(
        guard_acc: &TestAccount,
        clock: &TestAccount,
        data: &[u8],
    ) -> Result<(), pinocchio::error::ProgramError> {
        let accounts = [guard_acc.view, clock.view];
        process_instruction(&PROGRAM_ID, &accounts, data)
    }

    #[test]
    fn tick_cosigned_breach_stores_pending_without_advancing_nonce() {
        let guard_data = make_guard(AuthorityRequirement::CoSigned, 0, 0, 0);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        let clock = clock_account(20); // fresh: 20 - 0 <= 25

        // price 48m: pnl = 100m*(48m-50m)/1m = -200m, equity -100m, breach.
        let result = run_tick(&guard, &clock, &tick_data(48_000_000, 1, 0));
        assert!(result.is_ok());

        let state = GuardState::from_bytes(guard.data()).unwrap();
        // §8.4: CoSigned stores the action as pending...
        assert!(state.pending.is_some());
        // ...and must NOT advance the nonce — only the owner's L1 confirm does.
        assert_eq!(state.nonce, 0);
        assert_eq!(state.last_check_slot, 20);
        assert_eq!(state.current_price, 48_000_000);
        assert!(!state.degraded);
    }

    #[test]
    fn tick_cosigned_jupiter_tp_builds_owner_signed_safety_net() {
        // Jupiter + CoSigned + a take-profit action: the guard must build the
        // owner-signed `instantCreateTpsl` safety-net and hold it as pending —
        // but never advance the nonce and never submit it (§8.4/§8.7).
        let mut guard_data = make_guard(AuthorityRequirement::CoSigned, VENUE_JUPITER, 0, 0);
        {
            let mut state = GuardState::from_bytes(&guard_data).unwrap();
            state.policy.take_profit = Some(55_000_000);
            state.write_into(&mut guard_data).unwrap();
        }
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        let clock = clock_account(20);

        // price 58m (> TP 55m): take-profit fires; equity 900m > req, TP wins.
        let result = run_tick(&guard, &clock, &tick_data(58_000_000, 1, 0));
        assert!(result.is_ok());

        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.pending, Some(Action::TakeProfit));
        let px = state
            .pending_ix
            .expect("jupiter TP must build owner-signed data");
        // Expected nonce is nonce+1 and is NOT committed until owner confirms.
        assert_eq!(px.expected_nonce, 1);
        assert_eq!(state.nonce, 0, "nonce must not advance on CoSigned build");
        // The built data is the deterministic instantCreateTpsl payload.
        // clock_account leaves unix_timestamp at 0.
        let expected = build_tp_safety_net(55_000_000, 0).unwrap();
        assert_eq!(px.data, expected);
    }

    #[test]
    fn confirm_commits_nonce_and_clears_pending_ix() {
        // Guard with a pending owner-signed Jupiter instruction (expected 43).
        let mut guard_data = make_guard(AuthorityRequirement::CoSigned, VENUE_JUPITER, 42, 0);
        {
            let mut state = GuardState::from_bytes(&guard_data).unwrap();
            state.pending_ix = Some(PendingIx {
                expected_nonce: 43,
                data: [7u8; PENDING_IX_DATA_LEN],
            });
            state.write_into(&mut guard_data).unwrap();
        }
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        // Owner signer must match the guard's venue_owner, which make_guard
        // sets to [9u8; 32] — rebuild the owner account with that key.
        let owner = TestAccount::new(
            Address::new_from_array([9u8; 32]),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );

        // ConfirmYes discriminator = 9; no payload bytes needed.
        let data = [9u8];
        let accounts = [guard.view, owner.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &data);
        assert!(result.is_ok());

        let state = GuardState::from_bytes(guard.data()).unwrap();
        // §8.4 — the nonce commits only on the owner's L1 confirm.
        assert_eq!(state.nonce, 43);
        assert_eq!(state.pending_ix, None);
        assert_eq!(state.pending, None);
    }

    #[test]
    fn confirm_rejects_non_owner() {
        let mut guard_data = make_guard(AuthorityRequirement::CoSigned, VENUE_JUPITER, 42, 0);
        {
            let mut state = GuardState::from_bytes(&guard_data).unwrap();
            state.pending_ix = Some(PendingIx {
                expected_nonce: 43,
                data: [7u8; PENDING_IX_DATA_LEN],
            });
            state.write_into(&mut guard_data).unwrap();
        }
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        // Stranger signs — key != venue_owner ([9u8;32]).
        let stranger = TestAccount::new(
            Address::new_from_array([99u8; 32]),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );

        let accounts = [guard.view, stranger.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &[9u8]);
        assert_eq!(result, Err(WickError::SignerKeyMismatch.into()));
        // Nonce untouched.
        let state = GuardState::from_bytes(&guard_data).unwrap();
        assert_eq!(state.nonce, 42);
        assert!(state.pending_ix.is_some());
    }

    #[test]
    fn confirm_rejects_missing_signer_flag() {
        let mut guard_data = make_guard(AuthorityRequirement::CoSigned, VENUE_JUPITER, 42, 0);
        {
            let mut state = GuardState::from_bytes(&guard_data).unwrap();
            state.pending_ix = Some(PendingIx {
                expected_nonce: 43,
                data: [7u8; PENDING_IX_DATA_LEN],
            });
            state.write_into(&mut guard_data).unwrap();
        }
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        let owner_acc = TestAccount::new(
            Address::new_from_array([9u8; 32]),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            false, // correct key but is_signer = false
            false,
        );
        let accounts = [guard.view, owner_acc.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &[9u8]);
        assert_eq!(result, Err(WickError::MissingOwnerAuthority.into()));
    }

    #[test]
    fn confirm_rejects_when_nothing_pending() {
        let guard_data = make_guard(AuthorityRequirement::CoSigned, VENUE_JUPITER, 42, 0);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        let owner = TestAccount::new(
            Address::new_from_array([9u8; 32]),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );
        let accounts = [guard.view, owner.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &[9u8]);
        assert_eq!(result, Err(WickError::NoPendingConfirm.into()));
    }

    #[test]
    fn tick_healthy_updates_snapshot_no_action() {
        let guard_data = make_guard(AuthorityRequirement::CoSigned, 0, 0, 0);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        let clock = clock_account(20);

        // price 55m: equity 600m > req 5m — not liquidatable, TP unset.
        let result = run_tick(&guard, &clock, &tick_data(55_000_000, 1, 0));
        assert!(result.is_ok());

        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert!(state.pending.is_none());
        assert_eq!(state.current_price, 55_000_000);
        assert_eq!(state.last_check_slot, 20);
        assert_eq!(state.nonce, 0);
    }

    #[test]
    fn tick_replayed_nonce_is_noop() {
        let guard_data = make_guard(AuthorityRequirement::CoSigned, 0, 5, 0);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        let clock = clock_account(20);

        // Breach price but nonce 5 == last nonce → hard reject (§8.2).
        let result = run_tick(&guard, &clock, &tick_data(48_000_000, 5, 0));
        assert!(result.is_ok());

        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert!(state.pending.is_none());
        assert_eq!(state.nonce, 5);
        // Snapshot still refreshes so the guard never protects against
        // stale prices.
        assert_eq!(state.current_price, 48_000_000);
    }

    #[test]
    fn tick_stale_ticks_flip_degraded_after_three_then_recover() {
        let guard_data = make_guard(AuthorityRequirement::CoSigned, 0, 0, 0);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);

        // Three stale ticks — each arrives >25 slots after the previous check
        // (a glitching stream). First two do nothing, the third flips degraded.
        for slot in [100u64, 130, 160] {
            let clock = clock_account(slot);
            let result = run_tick(&guard, &clock, &tick_data(48_000_000, 1, 0));
            assert!(result.is_ok());
        }
        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.stale_streak, 3);
        assert!(state.degraded);
        // Stale ticks must never dispatch an action.
        assert!(state.pending.is_none());

        // A tick arriving within 25 slots of the last check is fresh and
        // clears the streak and the degraded flag (§8.1.3).
        let clock = clock_account(165); // 165 - 160 = 5 → fresh
        let result = run_tick(&guard, &clock, &tick_data(48_000_000, 1, 0));
        assert!(result.is_ok());
        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.stale_streak, 0);
        assert!(!state.degraded);
    }

    #[test]
    fn tick_autonomous_unsupported_venue_escalates() {
        // Autonomous regime but venue 0 (no adapter) — the venue executor
        // rejects, and the guard escalates to manual review rather than
        // silently no-opping (§8.2 #4).
        let guard_data = make_guard(AuthorityRequirement::Autonomous, 0, 0, 0);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        let clock = clock_account(20);

        let result = run_tick(&guard, &clock, &tick_data(48_000_000, 1, 0));
        assert!(result.is_ok());

        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.pending, Some(Action::EscalateManualReview));
        // Nonce NOT committed — the venue action never landed.
        assert_eq!(state.nonce, 0);
    }

    #[test]
    fn tick_drift_missing_venue_accounts_rejected() {
        // Autonomous Drift venue — the tick passes no venue adapter accounts
        // (state/user/authority + remaining). The adapter must reject rather
        // than silently no-op or advance the nonce. Use TakeProfit (fires
        // before the liquidity gate) so the reduce path is reached.
        let state = GuardState {
            venue: VENUE_DRIFT,
            venue_owner: [9u8; 32],
            co_authority: [5u8; 32],
            authority_req: AuthorityRequirement::Autonomous,
            policy: VenuePolicy {
                maintenance_bps: 500,
                trigger_buffer_bps: 500,
                fee_bps: 10,
                authority: AuthorityRequirement::Autonomous,
                caps: ActionCaps {
                    top_up_usd_per_action: u128::MAX,
                    partial_close_usd_per_action: u128::MAX,
                    daily_total_usd: u128::MAX,
                },
                take_profit: Some(49_000_000),
            },
            collateral: 100_000_000,
            size: 100_000_000,
            entry: 50_000_000,
            current_price: 49_000_000,
            nonce: 0,
            last_check_slot: 0,
            pending: None,
            pending_ix: None,
            degraded: false,
            stale_streak: 0,
            drift_market_index: 1,
            drift_subaccount_id: 0,
        };
        let mut guard_data = vec![0u8; GUARD_DATA_LEN];
        state.write_into(&mut guard_data).unwrap();
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        let clock = clock_account(20);

        // TakeProfit is the top-priority action (fires before the liquidity
        // gate), so it deterministically reaches the reduce path regardless of
        // solver reachability.
        let result = run_tick(&guard, &clock, &tick_data(49_000_000, 1, 0));
        assert_eq!(result, Err(WickError::InvalidInstruction.into()));

        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.nonce, 0);
    }

    #[test]
    fn tick_wrong_clock_address_rejected() {
        let guard_data = make_guard(AuthorityRequirement::CoSigned, 0, 0, 0);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        // Clock account at a non-sysvar address — must be rejected before any
        // state change.
        let bad_clock = TestAccount::new(
            addr(99),
            Address::new_from_array([0u8; 32]),
            0,
            &[0u8; 40],
            false,
            false,
        );
        let accounts = [guard.view, bad_clock.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &tick_data(48_000_000, 1, 0));
        assert_eq!(result, Err(WickError::InvalidInstruction.into()));
    }

    #[test]
    fn update_position_records_snapshot_owner_only() {
        let owner = addr(9);
        let guard_data = make_guard(AuthorityRequirement::CoSigned, 0, 0, 0);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);

        let mut data = vec![8u8];
        data.extend_from_slice(&300_000_000u128.to_le_bytes());
        data.extend_from_slice(&(-150_000_000i128).to_le_bytes());
        data.extend_from_slice(&55_000_000u128.to_le_bytes());

        let owner_acc = TestAccount::new(
            owner,
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );
        let accounts = [guard.view, owner_acc.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &data);
        assert!(result.is_ok());

        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.collateral, 300_000_000);
        assert_eq!(state.size, -150_000_000);
        assert_eq!(state.entry, 55_000_000);

        // Non-owner signer rejected, snapshot untouched.
        let stranger = TestAccount::new(
            addr(42),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );
        let accounts = [guard.view, stranger.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &data);
        assert_eq!(result, Err(WickError::SignerKeyMismatch.into()));
    }
}
