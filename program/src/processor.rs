//! Instruction dispatch and handlers.
//!
//! Account layout per instruction:
//!
//! * `InitGuard`    — [0] guard PDA (writable, created), [1] owner (signer),
//!   [2] payer (signer, writable), [3] rent sysvar. Payload = [bump | policy
//!   blob]; the PDA is derived from `b"guard"` || owner || bump.
//! * `DepositMargin`— [0] guard (writable, program-owned), [1] owner (signer),
//!   [2] route_config (readonly, program-owned — kill-switch check).
//!   Payload is the deposit `amount` (u128 LE). Credited to `collateral`.
//! * `WithdrawMargin`— [0] guard (writable, program-owned), [1] owner
//!   (signer), [2] co_authority (signer), [3] route_config (readonly).
//!   Payload is `amount` (u128 LE). Enforces the §8.5 2-of-2 rule.
//! * `SetPaused`    — [0] route-config (writable, program-owned), [1] config
//!   authority (signer). Payload is `paused` (u8).
//! * `OnPriceTick`  — [0] guard (writable), [1] clock, [2] route_config
//!   (readonly), [3] Pyth `PriceUpdateV2` (readonly, program-owned — the
//!   authoritative price source), [4..] venue adapter accounts.
//! * `UpdatePosition`— [0] guard (writable), [1] owner (signer),
//!   [2] route_config (readonly).
//! * `ConfirmYes`   — [0] guard (writable), [1] owner (signer),
//!   [2] route_config (readonly).
//! * `InitRouteConfig`— [0] config PDA (writable, created), [1] authority
//!   (signer), [2] payer (signer, writable), [3] rent sysvar.
//! * `CloseGuard`   — [0] guard (writable, program-owned), [1] owner (signer,
//!   writable — receives the rent refund). Payload = [bump].
//! * `SetRouteAuthority`— [0] route-config (writable, program-owned),
//!   [1] current authority (signer), [2] new authority (signer).
//! * `ReconcileVenue`— [0] guard (writable), [1] clock, [2] route_config
//!   (readonly), [3] venue position account (readonly, venue-owned). Payload is
//!   the reconcile `nonce` (u64 LE). Permissionless: the caller chooses *when*
//!   the guard looks at the venue, never *what it sees*.
//! * `InitMarginWallet`— [0] wallet PDA (writable, created), [1] guard
//!   (writable), [2] owner (signer), [3] payer (signer, writable), [4] rent
//!   sysvar, [5] route_config (readonly). Payload = [bump]; the PDA is derived
//!   from `b"margin"` || venue_owner || bump.
//! * `FundMarginWallet`— [0] wallet (writable, program-owned), [1] guard
//!   (readonly), [2] owner (signer, writable), [3] rent sysvar,
//!   [4] route_config (readonly). Payload is `amount` in **lamports** (u128 LE).
//! * `WithdrawMarginWallet`— [0] wallet (writable, program-owned), [1] guard
//!   (readonly), [2] owner (signer, writable), [3] co_authority (signer),
//!   [4] rent sysvar, [5] route_config (readonly). Payload is `amount` in
//!   lamports (u128 LE). Enforces the §8.5 2-of-2 rule.

use pinocchio::error::ProgramResult;
use pinocchio::instruction::{cpi::Signer, seeds};
use pinocchio::sysvars::clock::Clock;
use pinocchio::sysvars::rent::Rent;
use pinocchio::{AccountView, Address};

use pinocchio_system::instructions::{CreateAccount, Transfer};

use crate::account::{
    GuardState, PendingIx, WalletState, ACCOUNT_VERSION, GUARD_DATA_LEN,
    PENDING_IX_JUPITER_DEFENSIVE_CLOSE, PENDING_IX_JUPITER_TPSL, RECONCILE_DIVERGED,
    RECONCILE_NEVER, ROUTE_CONFIG_LEN, WALLET_DATA_LEN,
};
use crate::delegation;
use crate::drift::{
    read_user_position, verify_user_account, DriftPlaceOrderAccounts, ReduceDirection,
    ReduceOrderParams, VENUE_DRIFT,
};
use crate::error::WickError;
use crate::instruction::WickInstruction;
use crate::jupiter::{build_defensive_close, build_tp_safety_net, VENUE_JUPITER};
use crate::pyth::{pyth_price_to_scale6, read_price_no_older_than, SOL_USD_FEED_ID};
use crate::state::{
    accept_tick, action_daily_usd, guard_act, reconcile_verdict, roll_daily_epoch, select_action,
    track_tick_freshness, Action, ActionCaps, AuthorityRequirement, DispatchRegime, HealthSnapshot,
    RouteConfig, VenuePolicy, BPS_DENOM, SCALE,
};

const GUARD_SEED: &[u8] = b"guard";
const ROUTE_CONFIG_SEED: &[u8] = b"route_config";
/// Seed prefix of the 2-of-2 margin-wallet PDA: `b"margin" || venue_owner`.
const MARGIN_WALLET_SEED: &[u8] = b"margin";

/// How stale the price backing a co-signed defensive close may be before the
/// guard refuses to build it (§8.7). A stop level is only meaningful relative to
/// the mark it was derived from: handing the owner an instruction anchored to a
/// minute-old price invites them to sign a stop that is already through the
/// market. Matches `PYTH_MAX_AGE_SECS` — the same freshness the price itself is
/// gated on, not a looser second standard.
const DEFENSIVE_CLOSE_TTL_SECS: i64 = 60;

/// Pyth pull-oracle gating (hours.md §7.1): a tick is only priced against a
/// `PriceUpdateV2` no older than this many seconds, whose confidence is within
/// this many basis points of the price. Tighter than the accessor's unit-test
/// defaults; the guard never silently accepts a dead or wildly uncertain price.
const PYTH_MAX_AGE_SECS: u64 = 60;
const PYTH_MAX_CONF_BPS: u64 = 150;

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

/// Check the RouteConfig kill-switch. Must be called at the top of every
/// state-mutating instruction (§7). Returns `WickError::Paused` if the
/// program is paused.
fn check_not_paused(config_acc: &AccountView, program_id: &Address) -> Result<(), WickError> {
    if !config_acc.owned_by(program_id) {
        return Err(WickError::WrongAccountOwner);
    }
    let data = config_acc
        .try_borrow()
        .map_err(|_| WickError::NotInitialized)?;
    let cfg = RouteConfig::from_bytes(&data).map_err(|_| WickError::NotInitialized)?;
    if cfg.paused {
        return Err(WickError::Paused);
    }
    Ok(())
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
    } else {
        // Re-init guard. The runtime only re-derives the PDA from the seeds
        // during the `CreateAccount` CPI, so an already-funded account skips
        // that check: without this branch an attacker could pass a *victim's*
        // guard account with their own key as `owner` and overwrite its state
        // (resetting nonce and collateral). An initialized account is refused;
        // a live guard is only ever mutated by its own handlers.
        if !guard.owned_by(program_id) {
            return Err(WickError::WrongAccountOwner.into());
        }
        let data = guard.try_borrow().map_err(|_| WickError::NotInitialized)?;
        if data.len() != GUARD_DATA_LEN || data[0] == ACCOUNT_VERSION {
            return Err(WickError::AlreadyInitialized.into());
        }
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
        last_check_ts: 0,
        // A fresh guard starts with an unspent budget. The epoch anchors at 0
        // rather than the current slot so the first tick rolls it forward and
        // stamps a real start; enrolling does not consume any of day one.
        daily_spent_usd: 0,
        daily_epoch_start_ts: 0,
        pending: None,
        pending_ix: None,
        degraded: false,
        stale_streak: 0,
        drift_market_index,
        drift_subaccount_id,
        // §8.3 — no reconcile has run against the venue yet. `NeverReconciled`
        // is deliberately not `Diverged`: a guard that has never been checked is
        // not evidence of a mismatch, and starting disarmed would make every
        // fresh guard useless until a cranker happened to reconcile it.
        venue_size: 0,
        venue_collateral: 0,
        reconcile_ts: 0,
        reconcile_nonce: 0,
        reconcile_status: RECONCILE_NEVER,
        // No margin wallet until `InitMarginWallet` creates one and stamps its
        // bump here. 0 means "no reserve linked", which gates TopUp (§8.5).
        margin_wallet_bump: 0,
    };
    store_guard(guard, &state)?;
    Ok(())
}

fn deposit_margin(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let (guard, owner, route_config) = split_3(accounts)?;
    check_not_paused(route_config, program_id)?;
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
    let (guard, owner, co_authority, route_config) = split_4(accounts)?;
    check_not_paused(route_config, program_id)?;
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

/// Close a guard: refund its rent to the owner and zero the account.
///
/// The guard PDA is a pure function of `b"guard" || owner || bump`, so an owner
/// gets exactly one guard address, forever. Without this handler a guard whose
/// bytes no longer decode under the current layout is a tombstone at that one
/// address: `init_guard`'s re-init branch refuses an account whose version badge
/// already matches, every other handler refuses one that does not decode, and
/// the rent is unrecoverable. This is the escape hatch, and it re-arms itself on
/// every future layout change.
///
/// Deliberately **does not** decode the guard. The account that most needs
/// closing is the one that cannot be decoded, so authority is proven the only
/// way that survives a broken layout: the PDA is re-derived from the signing
/// owner's key and must equal the account being closed. That is the same
/// binding `init_guard` gets from the runtime during `CreateAccount`.
///
/// Also **not** gated on the kill switch. A pause exists to stop the guard from
/// acting; leaving the owner's rent trapped for the duration is not part of that
/// and would reintroduce the lockout this instruction removes. It moves no
/// value except the owner's own rent, back to the owner.
///
/// Account layout: [0] guard (writable, program-owned), [1] owner (signer,
/// writable — receives the rent refund).
/// Data (after discriminator): [0] bump.
fn close_guard(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let (guard, owner) = split_2(accounts)?;
    if !owner.is_signer() {
        return Err(WickError::MissingOwnerAuthority.into());
    }
    // A delegated guard is owned by the Delegation Program, not us; undelegate
    // it first. Checking here turns that into a clear error instead of a
    // runtime "spent from an account it does not own" failure.
    if !guard.owned_by(program_id) {
        return Err(WickError::WrongAccountOwner.into());
    }
    let bump = *data.get(1).ok_or(WickError::InvalidInstruction)?;

    let owner_key = owner.address().to_bytes();
    let expected = Address::create_program_address(&[GUARD_SEED, &owner_key, &[bump]], program_id)
        .map_err(|_| WickError::InvalidPda)?;
    if guard.address() != &expected {
        return Err(WickError::InvalidPda.into());
    }

    // Lamports must leave the account before it is closed or the instruction
    // ends unbalanced. The refund goes to the owner, who paid the rent.
    let refund = guard.lamports();
    owner.set_lamports(
        owner
            .lamports()
            .checked_add(refund)
            .ok_or(WickError::MathOverflow)?,
    );
    guard.set_lamports(0);
    guard.close()?;
    Ok(())
}

/// Rotate the RouteConfig kill-switch authority.
///
/// The authority is stamped once at `InitRouteConfig` and, until now, could
/// never change: a lost or compromised key meant the program could never be
/// paused again, or could be paused by someone who should no longer be able to.
///
/// The incoming authority must **sign**. Rotating a kill switch to a mistyped
/// address disables it permanently — the same one-way trap `CloseGuard` exists
/// to undo, except a RouteConfig has no second address to fall back on. A
/// signature proves the key exists and is controlled. A multisig can satisfy it.
///
/// Account layout: [0] route_config (writable, program-owned), [1] current
/// authority (signer), [2] new authority (signer).
/// Data (after discriminator): none — the new authority is account [2].
fn set_route_authority(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let (config, authority, new_authority) = split_3(accounts)?;
    if !config.owned_by(program_id) {
        return Err(WickError::WrongAccountOwner.into());
    }
    if !authority.is_signer() || !new_authority.is_signer() {
        return Err(WickError::Unauthorized.into());
    }

    let mut cfg =
        RouteConfig::from_bytes(&config.try_borrow().map_err(|_| WickError::NotInitialized)?)
            .map_err(|_| WickError::NotInitialized)?;
    if authority.address() != &Address::from(cfg.authority) {
        return Err(WickError::Unauthorized.into());
    }
    // `paused` is carried through untouched: a rotation during an incident must
    // not quietly un-pause the program.
    cfg.authority = new_authority.address().to_bytes();
    {
        let mut out = config
            .try_borrow_mut()
            .map_err(|_| WickError::NotInitialized)?;
        cfg.write_into(&mut out)
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
    let (guard, owner, route_config) = split_3(accounts)?;
    check_not_paused(route_config, program_id)?;
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
    // §8.3 — the model just changed, so the previous reconcile verdict no longer
    // describes it. Back to `NeverReconciled` rather than `Converged`: the owner
    // asserting a new snapshot is not evidence the venue agrees, and claiming
    // convergence the guard never observed would be the exact fiction
    // reconciliation exists to prevent. This is also the owner's way out of a
    // `Diverged` freeze — they correct the model, the guard re-arms, and the
    // next `ReconcileVenue` re-checks it against the venue's own bytes.
    //
    // `venue_size`/`venue_collateral`/`reconcile_ts` are left as they were: they
    // are a timestamped observation of the venue, still true as of that stamp,
    // and the console renders them alongside it.
    state.reconcile_status = RECONCILE_NEVER;
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
/// **Restriction (§8.4/§8.7).** Only `VENUE_JUPITER` builds a `pending_ix`, and
/// only for the two actions Jupiter's TP/SL instruction can express: `TakeProfit`
/// (tag `PENDING_IX_JUPITER_TPSL`) and `PartialClose` (tag
/// `PENDING_IX_JUPITER_DEFENSIVE_CLOSE`). Both are confirmable and both commit
/// the nonce the same way — the tag records *which* instruction the owner was
/// handed, so a console cannot offer a take-profit button for a stop. A CoSigned
/// guard on `VENUE_NONE` or `VENUE_DRIFT`, or on Jupiter with a top-up or an
/// escalation, records `pending` for the dashboard but produces nothing for the
/// owner to sign — those are advisory and must be actioned at the venue. This
/// path fails closed with `ConfirmUnsupportedForVenue` rather than committing a
/// nonce for work that never happened: advancing it would mark the breach
/// handled and disarm the guard against the next genuine one.
///
/// Account layout: [0] guard (writable, program-owned), [1] owner (signer).
/// Data (after discriminator): none — the pending instruction is on the guard.
fn confirm_pending(program_id: &Address, accounts: &[AccountView], _data: &[u8]) -> ProgramResult {
    let (guard, owner, route_config) = split_3(accounts)?;
    check_not_paused(route_config, program_id)?;
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
    let px = match state.pending_ix {
        Some(px) => px,
        // Distinguish "nothing is pending" from "something is pending but this
        // venue cannot produce a confirmable instruction" — otherwise the owner
        // sees the dashboard show a pending action and gets a bare
        // `NoPendingConfirm` with no way to tell which case they are in.
        None if state.pending.is_some() => return Err(WickError::ConfirmUnsupportedForVenue.into()),
        None => return Err(WickError::NoPendingConfirm.into()),
    };
    // §8.2 — the co-signed action lands now, so it charges the daily budget
    // now. Recomputed from the stored pending action rather than carried on
    // `PendingIx`, so the figure reflects the notional at confirm time.
    //
    // This is load-bearing for the defensive close (`PENDING_IX_JUPITER_-
    // DEFENSIVE_CLOSE`): it is a `PartialClose`, so confirming one spends
    // notional against the daily cap exactly as an autonomous reduce would.
    // Without the charge, a co-signed venue could be walked past its daily
    // budget one owner signature at a time. A take-profit charges nothing, so
    // the older Jupiter pairing is unaffected.
    //
    // No epoch rollover here: `Confirm` takes no clock account, and adding one
    // would change the instruction's account layout for every existing caller.
    // Charging against a possibly-expired epoch can only *under*-allow, never
    // over-allow, so the omission fails closed. The next tick rolls it.
    let notional = state
        .size
        .unsigned_abs()
        .checked_mul(state.current_price)
        .ok_or(WickError::MathOverflow)?
        .checked_div(SCALE)
        .ok_or(WickError::MathOverflow)?;
    let charge = state
        .pending
        .map(|a| action_daily_usd(a, notional))
        .unwrap_or(0);
    if !state
        .policy
        .caps
        .within_daily(state.daily_spent_usd, charge)
    {
        return Err(WickError::OverPolicyCap.into());
    }
    state.daily_spent_usd = state.daily_spent_usd.saturating_add(charge);

    // §8.4 — the nonce commits only when the owner confirms on L1.
    state.nonce = px.expected_nonce;
    state.pending_ix = None;
    state.pending = None;
    store_guard(guard, &state)?;
    Ok(())
}

// -------------------------------------------------------------------------
// §8.3 Venue reconciliation
// -------------------------------------------------------------------------

/// Reconcile the guard's model of the position against the venue's own account.
///
/// Everything else in this program trusts `state.size` — the solver sizes orders
/// from it, the health math prices it, the caps meter it. Until now the only
/// writer was the owner's `UpdatePosition`, so a position that moved at the venue
/// (a manual partial close, a liquidation, a fill the guard never saw) left the
/// guard confidently acting on a number that was no longer true.
///
/// This reads the venue's bytes and records what it found. Three properties make
/// it safe to leave open to anyone:
///
/// * **The number is never supplied by the caller.** The payload carries a nonce
///   and nothing else; size and collateral are decoded out of the venue's own
///   account, which is verified to be the account this guard watches
///   (`verify_user_account` re-derives it from the guard's `venue_owner` and
///   sub-account, and requires the venue program to own it).
/// * **Replays cannot re-apply an old snapshot.** `reconcile_nonce` must strictly
///   increase, so a captured transaction cannot roll a newer observation back to
///   an older one.
/// * **It moves no value and grants no authority.** The worst a hostile caller can
///   do is pay a fee to tell the guard the truth.
///
/// A `Diverged` verdict is *recorded, not raised*: returning an error would roll
/// the write back, and a divergence nobody can persist is a divergence nobody
/// acts on. `on_price_tick` reads the stored status and refuses to execute.
///
/// Account layout: [0] guard (writable, program-owned), [1] clock sysvar
/// (readonly), [2] route_config (readonly, program-owned), [3] venue position
/// account (readonly, owned by the venue program).
/// Data (after discriminator): [0..8] reconcile nonce (u64 LE).
fn reconcile_venue(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let (guard, clock, route_config) = split_3(accounts)?;
    let venue_account = accounts.get(3).ok_or(WickError::InvalidInstruction)?;
    check_not_paused(route_config, program_id)?;
    let now = Clock::from_account_view(clock)
        .map_err(|_| WickError::InvalidInstruction)?
        .unix_timestamp;

    let payload = data.get(1..).ok_or(WickError::InvalidInstruction)?;
    if payload.len() != 8 {
        return Err(WickError::InvalidInstruction.into());
    }
    let nonce = u64::from_le_bytes(payload[0..8].try_into().unwrap());

    let mut state = load_guard(guard, program_id)?;
    // Strictly increasing, checked before any venue work: a replayed or duplicate
    // reconcile is cheap to reject and must not overwrite a newer snapshot.
    if nonce <= state.reconcile_nonce {
        return Err(WickError::ReconcileStale.into());
    }
    // Only the autonomous tier has a position account to read. A Jupiter guard's
    // position lives behind keeper infrastructure the program cannot decode, and
    // guessing at it would be worse than admitting the gap.
    if state.venue != VENUE_DRIFT {
        return Err(WickError::UnsupportedVenueAction.into());
    }

    verify_user_account(venue_account, &state.venue_owner, state.drift_subaccount_id)?;
    let venue_data = venue_account
        .try_borrow()
        .map_err(|_| WickError::NotInitialized)?;
    let observed = read_user_position(&venue_data, state.drift_market_index)?;

    state.reconcile_status = reconcile_verdict(state.size, observed.size).to_byte();
    state.venue_size = observed.size;
    state.venue_collateral = observed.collateral;
    state.reconcile_ts = now;
    state.reconcile_nonce = nonce;
    store_guard(guard, &state)?;
    Ok(())
}

// -------------------------------------------------------------------------
// §8.5 Margin wallet — a real, 2-of-2 lamport reserve
// -------------------------------------------------------------------------

/// Re-derive the margin wallet PDA for a guard and check the supplied account is
/// it. Accepting a foreign wallet would let a guard credit itself from value its
/// owner does not control, or drain a wallet belonging to somebody else.
fn verify_margin_wallet(
    wallet: &AccountView,
    program_id: &Address,
    venue_owner: &[u8; 32],
    bump: u8,
) -> Result<(), WickError> {
    if !wallet.owned_by(program_id) {
        return Err(WickError::WrongAccountOwner);
    }
    let expected =
        Address::create_program_address(&[MARGIN_WALLET_SEED, venue_owner, &[bump]], program_id)
            .map_err(|_| WickError::InvalidPda)?;
    if wallet.address() != &expected {
        return Err(WickError::MarginWalletMismatch);
    }
    Ok(())
}

/// The reserve invariant (§8.5): the PDA's lamports must cover its own rent plus
/// every lamport it claims to hold on the owner's behalf.
///
/// Without it `balance` is just a number again — the thing this whole instruction
/// family exists to stop. Checked *after* every mutation, on both the funding and
/// the withdrawing path, so neither direction can leave the wallet claiming value
/// it does not have.
fn check_wallet_backed(
    wallet: &AccountView,
    rent: &AccountView,
    balance: u128,
) -> Result<(), WickError> {
    let minimum = Rent::from_account_view(rent)
        .map_err(|_| WickError::InvalidInstruction)?
        .try_minimum_balance(WALLET_DATA_LEN)
        .map_err(|_| WickError::MathOverflow)?;
    let backing = u128::from(
        wallet
            .lamports()
            .checked_sub(minimum)
            .ok_or(WickError::InsufficientMarginWallet)?,
    );
    if backing < balance {
        return Err(WickError::InsufficientMarginWallet);
    }
    Ok(())
}

/// Create the guard's margin wallet and link it.
///
/// Account layout: [0] wallet PDA (writable, created), [1] guard (writable,
/// program-owned), [2] owner (signer), [3] payer (signer, writable), [4] rent
/// sysvar, [5] route_config (readonly, program-owned).
/// Data (after discriminator): [0] wallet bump.
fn init_margin_wallet(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let (wallet, guard, owner, payer) = split_4(accounts)?;
    let rent = accounts.get(4).ok_or(WickError::InvalidInstruction)?;
    let route_config = accounts.get(5).ok_or(WickError::InvalidInstruction)?;
    check_not_paused(route_config, program_id)?;
    if !owner.is_signer() || !payer.is_signer() {
        return Err(WickError::MissingOwnerAuthority.into());
    }
    let bump = *data.get(1).ok_or(WickError::InvalidInstruction)?;

    let mut state = load_guard(guard, program_id)?;
    if owner.address() != &Address::from(state.venue_owner) {
        return Err(WickError::SignerKeyMismatch.into());
    }

    let bump_bytes = [bump];
    let seeds = seeds!(MARGIN_WALLET_SEED, &state.venue_owner[..], &bump_bytes);
    if wallet.lamports() == 0 {
        CreateAccount::with_minimum_balance(
            payer,
            wallet,
            WALLET_DATA_LEN as u64,
            program_id,
            Some(rent),
        )?
        .invoke_signed(&[Signer::from(&seeds)])?;
    } else {
        // Same trap as `init_guard`'s re-init branch: the runtime only re-derives
        // the PDA during `CreateAccount`, so a funded account skips that check.
        // Re-initializing a live wallet would zero a `balance` the owner funded.
        if !wallet.owned_by(program_id) {
            return Err(WickError::WrongAccountOwner.into());
        }
        let existing = wallet.try_borrow().map_err(|_| WickError::NotInitialized)?;
        if existing.len() != WALLET_DATA_LEN || existing[0] == ACCOUNT_VERSION {
            return Err(WickError::AlreadyInitialized.into());
        }
    }
    verify_margin_wallet(wallet, program_id, &state.venue_owner, bump)?;

    // The wallet inherits the guard's own 2-of-2 pair, so the exit path cannot be
    // widened by pointing a wallet at a friendlier co-authority.
    let ws = WalletState {
        owner: state.venue_owner,
        co_authority: state.co_authority,
        balance: 0,
    };
    {
        let mut out = wallet
            .try_borrow_mut()
            .map_err(|_| WickError::NotInitialized)?;
        ws.write_into(&mut out)
            .map_err(|_| WickError::NotInitialized)?;
    }

    state.margin_wallet_bump = bump;
    store_guard(guard, &state)?;
    Ok(())
}

/// Move lamports from the owner's wallet into the margin reserve.
///
/// Account layout: [0] wallet PDA (writable, program-owned), [1] guard
/// (readonly, program-owned), [2] owner (signer, writable), [3] rent sysvar,
/// [4] route_config (readonly, program-owned).
/// Data (after discriminator): amount in lamports (u128 LE, must fit u64).
fn fund_margin_wallet(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let (wallet, guard, owner, rent) = split_4(accounts)?;
    let route_config = accounts.get(4).ok_or(WickError::InvalidInstruction)?;
    check_not_paused(route_config, program_id)?;
    if !owner.is_signer() {
        return Err(WickError::MissingOwnerAuthority.into());
    }
    let amount = parse_amount(data)?;
    let lamports = u64::try_from(amount).map_err(|_| WickError::MathOverflow)?;

    let state = load_guard(guard, program_id)?;
    if owner.address() != &Address::from(state.venue_owner) {
        return Err(WickError::SignerKeyMismatch.into());
    }
    if state.margin_wallet_bump == 0 {
        return Err(WickError::MarginWalletMismatch.into());
    }
    verify_margin_wallet(
        wallet,
        program_id,
        &state.venue_owner,
        state.margin_wallet_bump,
    )?;

    let mut ws =
        WalletState::from_bytes(&wallet.try_borrow().map_err(|_| WickError::NotInitialized)?)
            .map_err(|_| WickError::NotInitialized)?;

    // Real lamports first, accounting second. A System transfer from a signing
    // owner needs no PDA signature and fails the whole instruction if the owner
    // cannot cover it, so `balance` is never incremented against a transfer that
    // did not happen.
    Transfer {
        from: owner,
        to: wallet,
        lamports,
    }
    .invoke()?;

    ws.balance = ws
        .balance
        .checked_add(amount)
        .ok_or(WickError::MathOverflow)?;
    {
        let mut out = wallet
            .try_borrow_mut()
            .map_err(|_| WickError::NotInitialized)?;
        ws.write_into(&mut out)
            .map_err(|_| WickError::NotInitialized)?;
    }
    check_wallet_backed(wallet, rent, ws.balance)?;
    Ok(())
}

/// Withdraw lamports out of the margin reserve — 2-of-2 (§8.5).
///
/// Account layout: [0] wallet PDA (writable, program-owned), [1] guard
/// (readonly, program-owned), [2] owner (signer, writable — receives the
/// lamports), [3] co_authority (signer), [4] rent sysvar, [5] route_config
/// (readonly, program-owned).
/// Data (after discriminator): amount in lamports (u128 LE, must fit u64).
fn withdraw_margin_wallet(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let (wallet, guard, owner, co_authority) = split_4(accounts)?;
    let rent = accounts.get(4).ok_or(WickError::InvalidInstruction)?;
    let route_config = accounts.get(5).ok_or(WickError::InvalidInstruction)?;
    check_not_paused(route_config, program_id)?;
    let amount = parse_amount(data)?;
    let lamports = u64::try_from(amount).map_err(|_| WickError::MathOverflow)?;

    let state = load_guard(guard, program_id)?;
    // The same §8.5 check `withdraw_margin` uses — value only leaves on two
    // signatures, and the wallet's own recorded pair must be the pair that signs.
    validate_withdraw(
        owner.is_signer(),
        owner.address(),
        &Address::from(state.venue_owner),
        co_authority.is_signer(),
        co_authority.address(),
        &Address::from(state.co_authority),
    )?;
    if state.margin_wallet_bump == 0 {
        return Err(WickError::MarginWalletMismatch.into());
    }
    verify_margin_wallet(
        wallet,
        program_id,
        &state.venue_owner,
        state.margin_wallet_bump,
    )?;

    let mut ws =
        WalletState::from_bytes(&wallet.try_borrow().map_err(|_| WickError::NotInitialized)?)
            .map_err(|_| WickError::NotInitialized)?;
    if amount > ws.balance {
        return Err(WickError::InsufficientMarginWallet.into());
    }
    if ws.owner != state.venue_owner || ws.co_authority != state.co_authority {
        return Err(WickError::MarginWalletMismatch.into());
    }

    // The wallet is program-owned, so lamports move by direct mutation rather
    // than a System CPI (System will not debit an account this program owns).
    wallet.set_lamports(
        wallet
            .lamports()
            .checked_sub(lamports)
            .ok_or(WickError::InsufficientMarginWallet)?,
    );
    owner.set_lamports(
        owner
            .lamports()
            .checked_add(lamports)
            .ok_or(WickError::MathOverflow)?,
    );

    ws.balance -= amount;
    {
        let mut out = wallet
            .try_borrow_mut()
            .map_err(|_| WickError::NotInitialized)?;
        ws.write_into(&mut out)
            .map_err(|_| WickError::NotInitialized)?;
    }
    // Rent must survive the withdrawal, or the account is closable by the
    // runtime and the remaining `balance` evaporates with it.
    check_wallet_backed(wallet, rent, ws.balance)?;
    Ok(())
}

// -------------------------------------------------------------------------
// §7.2 critical path — price tick
// -------------------------------------------------------------------------

/// OnPriceTick data layout (after the discriminator byte):
///   [0..8]  tick nonce (u64 LE — monotonic, supplied by the tick source)
///   [8]     guard PDA bump (needed to sign the autonomous venue CPI)
///
/// The price is NOT part of the tick payload: it is read from the Pyth
/// `PriceUpdateV2` account at index [3] and gated on feed id, staleness and
/// confidence (§7.1). A caller-supplied price is never trusted, so a cranker
/// cannot nudge the guard's health math by posting an arbitrary number.
///
/// Account layout:
///   [0]    guard (writable, program-owned)
///   [1]    clock sysvar (readonly)
///   [2]    route_config (readonly, program-owned — kill-switch check)
///   [3]    Pyth `PriceUpdateV2` (readonly, owned by the Pyth receiver program)
///   [4..]  venue adapter accounts (only consumed for an autonomous venue CPI)
fn on_price_tick(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let (guard, clock, route_config) = split_3(accounts)?;
    let pyth_account = accounts.get(3).ok_or(WickError::InvalidInstruction)?;
    check_not_paused(route_config, program_id)?;
    if !guard.owned_by(program_id) {
        return Err(WickError::WrongAccountOwner.into());
    }
    let clock_sv = Clock::from_account_view(clock).map_err(|_| WickError::InvalidInstruction)?;
    let now = clock_sv.unix_timestamp;

    let payload = data.get(1..).ok_or(WickError::InvalidInstruction)?;
    if payload.len() < 9 {
        return Err(WickError::InvalidInstruction.into());
    }
    let tick_nonce = u64::from_le_bytes(payload[0..8].try_into().unwrap());
    let bump = payload[8];

    // §7.1 — the authoritative price. The account must belong to the Pyth
    // receiver program and carry a Full-verified SOL/USD update, no staler
    // than `PYTH_MAX_AGE_SECS`, with confidence within `PYTH_MAX_CONF_BPS`.
    let oracle_data = pyth_account
        .try_borrow()
        .map_err(|_| WickError::NotInitialized)?;
    // SAFETY: `owner` is only read here; the account is not writable in this
    // instruction, so the reference cannot be invalidated by an `assign`/`close`.
    let pyth = read_price_no_older_than(
        &oracle_data,
        unsafe { pyth_account.owner() },
        &SOL_USD_FEED_ID,
        now,
        PYTH_MAX_AGE_SECS,
        PYTH_MAX_CONF_BPS,
    )?;
    let price = pyth_price_to_scale6(&pyth)?;

    let mut state = load_guard(guard, program_id)?;

    // `OnPriceTick` is permissionless (any cranker may drive it), so the tick
    // nonce is attacker-controlled. `select_action` only rejects nonces at or
    // *below* the committed one, which leaves a denial-of-service: one tick
    // carrying `u64::MAX` commits that nonce on the next landed action and
    // every genuine tick afterwards looks like a replay, silently disarming
    // the guard. The nonce may only ever step forward by one.
    if tick_nonce > state.nonce.saturating_add(1) {
        return Err(WickError::NonceOutOfOrder.into());
    }

    // §8.1.3 — reject stale ticks, tracking the degraded streak. A rejected
    // tick is not "do nothing this tick": it updates the streak/flag so the
    // guard can never silently protect against dead data. Wall-clock freshness
    // (`now`) rather than slot — see MAX_TICK_AGE_SECS: a slot means ~400ms on
    // base Solana and ~50ms on the ER, seconds mean the same on both.
    let fresh = accept_tick(now, state.last_check_ts);
    let (streak, degraded) = track_tick_freshness(state.stale_streak, fresh);
    state.stale_streak = streak;
    state.degraded = degraded;
    state.current_price = price;
    state.last_check_ts = now;

    if !fresh {
        store_guard(guard, &state)?;
        return Ok(());
    }

    // §8.2 — roll the daily budget before it is consulted, so a guard whose
    // epoch elapsed while it sat idle starts this tick with a fresh allowance.
    let (spent, epoch_start) =
        roll_daily_epoch(state.daily_spent_usd, state.daily_epoch_start_ts, now);
    state.daily_spent_usd = spent;
    state.daily_epoch_start_ts = epoch_start;

    // §8.2 — health → nonce → caps → action selection (§7.2 ordering).
    let health = HealthSnapshot {
        collateral: state.collateral,
        size: state.size,
        entry: state.entry,
        current_price: price,
    };
    let Some(action) = select_action(&health, &state.policy, tick_nonce, state.nonce, spent)?
    else {
        store_guard(guard, &state)?; // healthy — snapshot updated, nothing else
        return Ok(());
    };

    // USD this action commits against the daily budget, charged at the point it
    // actually lands — immediately for an autonomous execution, at the owner's
    // confirm for a co-signed one. Charging at selection time would let a
    // co-signed build the owner never signs burn the budget anyway.
    let notional = state
        .size
        .unsigned_abs()
        .checked_mul(price)
        .ok_or(WickError::MathOverflow)?
        .checked_div(SCALE)
        .ok_or(WickError::MathOverflow)?;
    let action_usd = action_daily_usd(action, notional);

    // §8.4 — two-regime authority dispatch.
    let (regime, expected_nonce) = guard_act(&state.policy, tick_nonce)?;
    // Venue adapter accounts start at index 4 (after guard, clock,
    // route_config, pyth oracle).
    let venue_accounts = accounts.get(4..).unwrap_or(&[]);
    match regime {
        // §8.3.4 — fail closed on a diverged model. The last reconcile found the
        // venue's own bytes out of tolerance of `state.size`, and every
        // autonomous order the guard would place is sized *from* `state.size`.
        // Executing here trades a number the venue has already contradicted: too
        // small and the breach is not cleared, too large and a `reduce_only`
        // order that should trim a position closes it outright. Escalate instead
        // and leave the nonce uncommitted, so the guard re-arms the moment a
        // fresh `ReconcileVenue` converges or the owner runs `UpdatePosition`.
        //
        // The price, freshness streak and daily epoch are already updated above,
        // so a diverged guard still reports live health — it just will not act.
        DispatchRegime::Autonomous if state.reconcile_status == RECONCILE_DIVERGED => {
            state.pending = Some(Action::EscalateManualReview);
        }
        // §8.5 — a TopUp the owner cannot fund is not an action, it is a
        // suggestion. The margin wallet is the only value the guard controls, and
        // `margin_wallet_bump == 0` means no reserve was ever linked. Surface
        // manual review rather than a top-up that has nothing behind it.
        //
        // This checks that a reserve *exists*, not that it covers the draw: the
        // wallet is not ER-delegated and is not in this instruction's account
        // list, so its balance is unreadable from here. Sufficiency is enforced
        // at `FundMarginWallet`/`WithdrawMarginWallet`, where lamports move.
        DispatchRegime::Autonomous
            if matches!(action, Action::TopUp { .. }) && state.margin_wallet_bump == 0 =>
        {
            state.pending = Some(Action::EscalateManualReview);
        }
        DispatchRegime::Autonomous => {
            match execute_autonomous(&state, action, venue_accounts, bump) {
                Ok(VenueOutcome::Executed) => {
                    // Nonce commits only when the venue action actually lands.
                    state.nonce = expected_nonce;
                    state.daily_spent_usd = state.daily_spent_usd.saturating_add(action_usd);
                    // The position just shrank on the venue, so the guard's
                    // model of it has to shrink too. Without this the guard
                    // still believes it holds the pre-reduce size, re-solves
                    // against the original notional on the very next tick, and
                    // under a sustained breach closes the whole position in a
                    // handful of ticks — each one a real order at the venue.
                    // Mirrors `execute_drift_autonomous`'s own arithmetic.
                    apply_executed_reduce(&mut state, action);
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
                let built = match action {
                    // The profit-taking leg: a trigger *above* market covering
                    // the whole position (§8.7).
                    Action::TakeProfit => Some((
                        PENDING_IX_JUPITER_TPSL,
                        build_tp_safety_net(
                            state.policy.take_profit.unwrap_or(state.current_price),
                            now,
                        )?,
                    )),
                    // §8.7 — the defensive leg. A CoSigned Jupiter guard used to
                    // record `PartialClose` for the dashboard and build nothing,
                    // so the one action that answers a live breach was the one
                    // the owner could not sign. Build a stop for exactly the
                    // notional the solver sized, anchored to this tick's verified
                    // price, and hold it pending — the guard still never signs.
                    Action::PartialClose { fraction_bps } => {
                        let close_usd = notional
                            .checked_mul(fraction_bps)
                            .ok_or(WickError::MathOverflow)?
                            / BPS_DENOM;
                        Some((
                            PENDING_IX_JUPITER_DEFENSIVE_CLOSE,
                            build_defensive_close(
                                // The trigger is this tick's verified mark, not
                                // some level further out: the breach is already
                                // live, so the honest instruction is "close this
                                // much, now", not "close it if things get worse".
                                price,
                                close_usd,
                                state.size < 0,
                                pyth.publish_time,
                                now,
                                DEFENSIVE_CLOSE_TTL_SECS,
                            )?,
                        ))
                    }
                    // A top-up adds collateral, which Jupiter's TP/SL
                    // instruction cannot express, and an escalation is by
                    // definition not automatable. Both stay advisory.
                    Action::TopUp { .. } | Action::EscalateManualReview => None,
                };
                if let Some((kind, data)) = built {
                    state.pending_ix = Some(PendingIx {
                        kind,
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

/// Update the guard's model of the position after an autonomous reduce landed.
///
/// A `PartialClose{fraction_bps}` shrinks `size` by that fraction in magnitude
/// (matching `execute_drift_autonomous`: `abs_size * fraction_bps / 10_000`,
/// floored to whole base units, so the residual always rounds in the guard's
/// favor — it models holding slightly *more* than the venue now holds). A
/// `TakeProfit` closes the watched position in full, so `size` goes to zero.
/// `TopUp`/`Escalate` never reach here — they cannot execute.
///
/// The `executed` reduce is the authoritative notification that exposure left
/// the venue; `current_price`, `entry` and `collateral` are left untouched.
fn apply_executed_reduce(state: &mut GuardState, action: Action) {
    match action {
        Action::TakeProfit => state.size = 0,
        Action::PartialClose { fraction_bps } => {
            let abs = state.size.unsigned_abs();
            let reduced = abs.saturating_mul(fraction_bps) / 10_000;
            // `size >= 0` is the same direction test `execute_drift_autonomous`
            // uses to pick the venue order direction.
            if state.size >= 0 {
                state.size = (abs.saturating_sub(reduced)) as i128;
            } else {
                state.size = -((abs.saturating_sub(reduced)) as i128);
            }
        }
        Action::TopUp { .. } | Action::EscalateManualReview => {}
    }
}

/// Outcome of an autonomous venue execution.
enum VenueOutcome {
    /// The venue action landed on-chain; the nonce may commit.
    Executed,
    /// No venue action is possible; surface manual review.
    Escalate,
}

/// Execute an action through the venue adapter in the autonomous tier. The
/// guard PDA signs as the position delegate/owner — that is what ER delegation
/// (§8.6) unlocks for sub-50ms execution.
///
/// `venue_accounts` is the tail of the instruction's account list starting
/// *after* the guard, clock, and route_config accounts.
fn execute_autonomous(
    state: &GuardState,
    action: Action,
    venue_accounts: &[AccountView],
    bump: u8,
) -> Result<VenueOutcome, WickError> {
    if state.venue == VENUE_JUPITER {
        // §8.7 — Jupiter is the co-signed tier. The guard cannot fake the
        // position owner's signature, so it never executes Jupiter
        // instructions autonomously. The safety-net TP/SL instruction data is
        // the owner's to sign (§8.4 CoSigned); autonomy here is not possible.
        return Err(WickError::UnsupportedVenueAction);
    }
    if state.venue != VENUE_DRIFT {
        // No autonomous venue adapter for this venue tag. Reject so the tick
        // escalates to manual review rather than silently no-oping.
        return Err(WickError::UnsupportedVenueAction);
    }
    execute_drift_autonomous(state, action, venue_accounts, bump)
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
    venue_accounts: &[AccountView],
    bump: u8,
) -> Result<VenueOutcome, WickError> {
    // Reduce fraction: a TakeProfit closes the watched size in full; a Partial
    // only shrinks the position by `fraction_bps` (already capped in §8.2).
    let fraction_bps: u128 = match action {
        Action::PartialClose { fraction_bps } => fraction_bps,
        Action::TakeProfit => 10_000,
        // §8.5 — a TopUp adds *collateral*, which on Drift means a `deposit` into
        // the quote spot market: an SPL token transfer out of a token account the
        // guard PDA owns, against the spot-market vault and its oracle. None of
        // those accounts are in this instruction's list and no deposit adapter is
        // wired, so the guard has no way to move the value.
        //
        // It escalates rather than pretending. The alternative — debiting the
        // margin wallet and crediting `state.collateral` — would be strictly
        // worse than doing nothing: the guard's health math would price a
        // position as rescued while the venue, which is the only party that can
        // liquidate it, saw no collateral arrive. `on_price_tick` already refuses
        // to route a TopUp here at all unless a funded reserve is linked, so the
        // escalation reaches an owner who can act on it.
        Action::TopUp { .. } | Action::EscalateManualReview => {
            return Err(WickError::UnsupportedVenueAction)
        }
    };

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
        WickInstruction::Delegate => delegation::process_delegate(program_id, accounts, &data[1..]),
        WickInstruction::CommitAndUndelegate => delegation::process_commit_and_undelegate(accounts),
        WickInstruction::Commit => delegation::process_commit(accounts),
        WickInstruction::OnPriceTick => on_price_tick(program_id, accounts, data),
        WickInstruction::UpdatePosition => update_position(program_id, accounts, data),
        WickInstruction::ConfirmYes => confirm_pending(program_id, accounts, data),
        WickInstruction::InitRouteConfig => init_route_config(program_id, accounts, data),
        WickInstruction::CloseGuard => close_guard(program_id, accounts, data),
        WickInstruction::SetRouteAuthority => set_route_authority(program_id, accounts),
        WickInstruction::ReconcileVenue => reconcile_venue(program_id, accounts, data),
        WickInstruction::InitMarginWallet => init_margin_wallet(program_id, accounts, data),
        WickInstruction::FundMarginWallet => fund_margin_wallet(program_id, accounts, data),
        WickInstruction::WithdrawMarginWallet => withdraw_margin_wallet(program_id, accounts, data),
    }
}

/// Create the singleton RouteConfig PDA.
///
/// Account layout: [0] config PDA (writable, created), [1] authority (signer),
/// [2] payer (signer, writable), [3] rent sysvar.
/// Data (after discriminator): [0] bump.
fn init_route_config(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let (config, authority, payer, rent) = split_4(accounts)?;
    if !authority.is_signer() || !payer.is_signer() {
        return Err(WickError::MissingOwnerAuthority.into());
    }
    let bump = *data.get(1).ok_or(WickError::InvalidInstruction)?;
    let bump_bytes = [bump];
    let seeds = seeds!(ROUTE_CONFIG_SEED, &bump_bytes);
    let signer = Signer::from(&seeds);

    if config.lamports() == 0 {
        let create_account = CreateAccount::with_minimum_balance(
            payer,
            config,
            ROUTE_CONFIG_LEN as u64,
            program_id,
            Some(rent),
        )?;
        create_account.invoke_signed(&[signer])?;
    } else {
        // Kill-switch takeover. `InitRouteConfig` is permissionless by design
        // (whoever creates the singleton becomes its authority), so re-running
        // it against the existing PDA would let anyone install themselves as
        // the pause authority and clear `paused`. Initialization is one-shot.
        if !config.owned_by(program_id) {
            return Err(WickError::WrongAccountOwner.into());
        }
        let data = config.try_borrow().map_err(|_| WickError::NotInitialized)?;
        if data.len() != ROUTE_CONFIG_LEN || data[0] == ACCOUNT_VERSION {
            return Err(WickError::AlreadyInitialized.into());
        }
    }
    let cfg = RouteConfig {
        authority: authority.address().to_bytes(),
        paused: false,
        _padding: [0u8; 31],
    };
    {
        let mut out = config
            .try_borrow_mut()
            .map_err(|_| WickError::NotInitialized)?;
        cfg.write_into(&mut out)
            .map_err(|_| WickError::NotInitialized)?;
    }
    Ok(())
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
        payload[0] = VENUE_DRIFT; // venue = Drift
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
        assert_eq!(venue, VENUE_DRIFT);
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
            last_check_ts: 0,
            pending: None,
            pending_ix: None,
            degraded: false,
            stale_streak: 0,
            drift_market_index: 0,
            drift_subaccount_id: 0,
            daily_spent_usd: 0,
            daily_epoch_start_ts: 0,
            venue_size: 0,
            venue_collateral: 0,
            reconcile_ts: 0,
            reconcile_nonce: 0,
            reconcile_status: RECONCILE_NEVER,
            margin_wallet_bump: 0,
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
        last_check_ts: i64,
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
            last_check_ts,
            pending: None,
            pending_ix: None,
            degraded: false,
            stale_streak: 0,
            drift_market_index: 0,
            drift_subaccount_id: 0,
            daily_spent_usd: 0,
            daily_epoch_start_ts: 0,
            venue_size: 0,
            venue_collateral: 0,
            reconcile_ts: 0,
            reconcile_nonce: 0,
            reconcile_status: RECONCILE_NEVER,
            margin_wallet_bump: 0,
        };
        let mut buf = vec![0u8; GUARD_DATA_LEN];
        state.write_into(&mut buf).unwrap();
        buf
    }

    /// OnPriceTick payload: [7, nonce:8, bump:1]. The price now comes from the
    /// Pyth `PriceUpdateV2` fixture, never the payload (security: no caller-
    /// supplied price).
    fn tick_data(nonce: u64, bump: u8) -> [u8; 10] {
        let mut d = [0u8; 10];
        d[0] = 7;
        d[1..9].copy_from_slice(&nonce.to_le_bytes());
        d[9] = bump;
        d
    }

    /// A Pyth `PriceUpdateV2` account carrying `price6` in Wick's 6-decimal
    /// scale, published at `publish_ts` with zero confidence, owned by the Pyth
    /// receiver program. Mirrors the golden layout in `pyth::tests`.
    /// With expo=-8, raw * 10^(expo+6) = raw * 10^-2 == price6 ⇒ raw = price6*100.
    ///
    /// `publish_ts` tracks the tick's clock rather than being pinned at 0:
    /// `read_price_no_older_than` rejects anything older than
    /// `PYTH_MAX_AGE_SECS`, so a fixture frozen at the unix epoch reads as
    /// decades stale against any realistic timestamp.
    fn pyth_price_account_at(price6: u128, publish_ts: i64) -> TestAccount {
        let mut data = vec![0u8; 200];
        data[..8].copy_from_slice(&crate::pyth::PRICE_UPDATE_V2_DISCRIMINATOR);
        data[40] = 1; // Full verification
        data[41..73].copy_from_slice(&crate::pyth::SOL_USD_FEED_ID);
        data[73..81].copy_from_slice(&((price6 * 100) as i64).to_le_bytes()); // raw (i64)
        data[81..89].copy_from_slice(&0u64.to_le_bytes()); // conf = 0
        data[89..93].copy_from_slice(&(-8i32).to_le_bytes()); // exponent
        data[93..101].copy_from_slice(&publish_ts.to_le_bytes()); // publish_time
        TestAccount::new(
            Address::new_from_array([3u8; 32]), // mock address
            crate::pyth::PYTH_RECEIVER_PROGRAM_ID,
            0,
            &data,
            false,
            false,
        )
    }

    /// A Clock sysvar account reporting `slot`. Address must be CLOCK_ID or
    /// `Clock::from_account_view` rejects it.
    /// Clock sysvar: slot at 0, then epoch_start_timestamp, epoch,
    /// leader_schedule_epoch, and `unix_timestamp` (i64) at 32.
    ///
    /// Both are driven off one axis so a test that means "10 units later" gets a
    /// coherent clock. Tick freshness reads `unix_timestamp` (§8.1), so leaving
    /// it at 0 while advancing only the slot would make every tick look
    /// 56-years stale.
    fn clock_account(ts: i64) -> TestAccount {
        let mut data = vec![0u8; 40];
        data[0..8].copy_from_slice(&(ts as u64).to_le_bytes());
        data[32..40].copy_from_slice(&ts.to_le_bytes());
        TestAccount::new(
            CLOCK_ID,
            Address::new_from_array([0u8; 32]),
            0,
            &data,
            false,
            false,
        )
    }

    fn route_config_account() -> TestAccount {
        let cfg = RouteConfig {
            authority: [0u8; 32],
            paused: false,
            _padding: [0u8; 31],
        };
        let mut data = vec![0u8; ROUTE_CONFIG_LEN];
        cfg.write_into(&mut data).unwrap();
        TestAccount::new(
            Address::new_from_array([2u8; 32]), // mock address
            PROGRAM_ID,
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

        let route_config = route_config_account();
        let accounts = [guard.view, owner_acc.view, route_config.view];
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

        let route_config = route_config_account();
        let accounts = [guard.view, stranger.view, route_config.view];
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

        let route_config = route_config_account();
        let accounts = [guard.view, owner_acc.view, route_config.view];
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

        let route_config = route_config_account();
        let accounts = [guard.view, owner_acc.view, co.view, route_config.view];
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

        let route_config = route_config_account();
        let accounts = [guard.view, owner_acc.view, co.view, route_config.view];
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

        let route_config = route_config_account();
        let accounts = [guard.view, owner_acc.view, co.view, route_config.view];
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

        let route_config = route_config_account();
        let accounts = [guard.view, owner_acc.view, route_config.view, co.view];
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
        let result = delegation::process_delegate(&PROGRAM_ID, &empty, &data[1..]);
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
    /// account; a fresh clock account is created per call. The price comes
    /// from the Pyth oracle fixture, not the payload.
    fn run_tick(
        guard_acc: &TestAccount,
        ts: i64,
        price: u128,
        data: &[u8],
    ) -> Result<(), pinocchio::error::ProgramError> {
        let clock = clock_account(ts);
        let route_config = route_config_account();
        // The oracle publishes at the tick's own timestamp — these tests are
        // about tick freshness, not oracle staleness, which has its own tests.
        let pyth = pyth_price_account_at(price, ts);
        let accounts = [guard_acc.view, clock.view, route_config.view, pyth.view];
        process_instruction(&PROGRAM_ID, &accounts, data)
    }

    #[test]
    fn tick_cosigned_breach_stores_pending_without_advancing_nonce() {
        let guard_data = make_guard(AuthorityRequirement::CoSigned, 0, 0, 0);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);

        // price 48m: pnl = 100m*(48m-50m)/1m = -200m, equity -100m, breach.
        let result = run_tick(&guard, 20, 48_000_000, &tick_data(1, 0));
        assert!(result.is_ok());

        let state = GuardState::from_bytes(guard.data()).unwrap();
        // §8.4: CoSigned stores the action as pending...
        assert!(state.pending.is_some());
        // ...and must NOT advance the nonce — only the owner's L1 confirm does.
        assert_eq!(state.nonce, 0);
        assert_eq!(state.last_check_ts, 20);
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

        // price 58m (> TP 55m): take-profit fires; equity 900m > req, TP wins.
        let result = run_tick(&guard, 20, 58_000_000, &tick_data(1, 0));
        assert!(result.is_ok());

        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.pending, Some(Action::TakeProfit));
        let px = state
            .pending_ix
            .expect("jupiter TP must build owner-signed data");
        // Expected nonce is nonce+1 and is NOT committed until owner confirms.
        assert_eq!(px.expected_nonce, 1);
        assert_eq!(state.nonce, 0, "nonce must not advance on CoSigned build");
        // The built data is the deterministic instantCreateTpsl payload, whose
        // expiry is anchored to the tick's `unix_timestamp` (20 above).
        let expected = build_tp_safety_net(55_000_000, 20).unwrap();
        assert_eq!(px.data, expected);
    }

    #[test]
    fn confirm_commits_nonce_and_clears_pending_ix() {
        // Guard with a pending owner-signed Jupiter instruction (expected 43).
        let mut guard_data = make_guard(AuthorityRequirement::CoSigned, VENUE_JUPITER, 42, 0);
        {
            let mut state = GuardState::from_bytes(&guard_data).unwrap();
            state.pending_ix = Some(PendingIx {
                kind: PENDING_IX_JUPITER_TPSL,
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
        let route_config = route_config_account();
        let accounts = [guard.view, owner.view, route_config.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &data);
        assert!(result.is_ok());

        let state = GuardState::from_bytes(guard.data()).unwrap();
        // §8.4 — the nonce commits only on the owner's L1 confirm.
        assert_eq!(state.nonce, 43);
        assert_eq!(state.pending_ix, None);
        assert_eq!(state.pending, None);
    }

    #[test]
    fn confirm_rejects_pending_with_no_owner_signed_instruction() {
        // §8.4/§8.7 — only VENUE_JUPITER + TakeProfit builds a `pending_ix`. A
        // CoSigned guard on any other venue records `pending` for the dashboard
        // but has nothing for the owner to sign. Confirming must fail closed
        // with a distinct error: committing the nonce here would mark the breach
        // handled and disarm the guard against the next genuine one.
        // venue 0 = no adapter (the frontend's VENUE_NONE); 3 = Drift.
        for venue in [0u8, VENUE_DRIFT] {
            let mut guard_data = make_guard(AuthorityRequirement::CoSigned, venue, 42, 0);
            {
                let mut state = GuardState::from_bytes(&guard_data).unwrap();
                state.pending = Some(Action::TopUp { amount: 1_000 });
                state.pending_ix = None;
                state.write_into(&mut guard_data).unwrap();
            }
            let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
            let owner = TestAccount::new(
                Address::new_from_array([9u8; 32]),
                Address::new_from_array([0u8; 32]),
                0,
                &[],
                true,
                false,
            );
            let route_config = route_config_account();
            let accounts = [guard.view, owner.view, route_config.view];
            let result = process_instruction(&PROGRAM_ID, &accounts, &[9u8]);
            assert_eq!(
                result,
                Err(WickError::ConfirmUnsupportedForVenue.into()),
                "venue {venue} should reject confirm"
            );

            // Nonce and pending action both untouched — the guard stays armed.
            let state = GuardState::from_bytes(guard.data()).unwrap();
            assert_eq!(state.nonce, 42);
            assert_eq!(state.pending, Some(Action::TopUp { amount: 1_000 }));
        }
    }

    #[test]
    fn confirm_with_nothing_pending_is_distinct_from_unsupported_venue() {
        // Empty guard: no pending action at all. This is the "nothing to do"
        // case and must stay distinguishable from the venue restriction above.
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
        let route_config = route_config_account();
        let accounts = [guard.view, owner.view, route_config.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &[9u8]);
        assert_eq!(result, Err(WickError::NoPendingConfirm.into()));
    }

    #[test]
    fn confirm_rejects_non_owner() {
        let mut guard_data = make_guard(AuthorityRequirement::CoSigned, VENUE_JUPITER, 42, 0);
        {
            let mut state = GuardState::from_bytes(&guard_data).unwrap();
            state.pending_ix = Some(PendingIx {
                kind: PENDING_IX_JUPITER_TPSL,
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

        let route_config = route_config_account();
        let accounts = [guard.view, stranger.view, route_config.view];
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
                kind: PENDING_IX_JUPITER_TPSL,
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
        let route_config = route_config_account();
        let accounts = [guard.view, owner_acc.view, route_config.view];
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
        let route_config = route_config_account();
        let accounts = [guard.view, owner.view, route_config.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &[9u8]);
        assert_eq!(result, Err(WickError::NoPendingConfirm.into()));
    }

    #[test]
    fn tick_healthy_updates_snapshot_no_action() {
        let guard_data = make_guard(AuthorityRequirement::CoSigned, 0, 0, 0);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);

        // price 55m: equity 600m > req 5m — not liquidatable, TP unset.
        let result = run_tick(&guard, 20, 55_000_000, &tick_data(1, 0));
        assert!(result.is_ok());

        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert!(state.pending.is_none());
        assert_eq!(state.current_price, 55_000_000);
        assert_eq!(state.last_check_ts, 20);
        assert_eq!(state.nonce, 0);
    }

    #[test]
    fn tick_replayed_nonce_is_noop() {
        let guard_data = make_guard(AuthorityRequirement::CoSigned, 0, 5, 0);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);

        // Breach price but nonce 5 == last nonce → hard reject (§8.2).
        let result = run_tick(&guard, 20, 48_000_000, &tick_data(5, 0));
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

        // A guard's first tick has nothing to measure against
        // (`last_check_ts == 0`), so it anchors the clock rather than counting as
        // stale. Without this the very first tick of every new guard would open a
        // streak at 1. Priced healthy (55m, not the 48m breach used below) so the
        // anchor itself has nothing to dispatch.
        assert!(run_tick(&guard, 100, 55_000_000, &tick_data(1, 0)).is_ok());
        assert_eq!(
            GuardState::from_bytes(guard.data()).unwrap().stale_streak,
            0
        );
        assert!(GuardState::from_bytes(guard.data())
            .unwrap()
            .pending
            .is_none());

        // Three stale ticks — each arrives >MAX_TICK_AGE_SECS after the previous
        // check (a glitching stream). First two do nothing, the third degrades.
        for ts in [130i64, 160, 190] {
            let result = run_tick(&guard, ts, 48_000_000, &tick_data(1, 0));
            assert!(result.is_ok());
        }
        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.stale_streak, 3);
        assert!(state.degraded);
        // Stale ticks must never dispatch an action.
        assert!(state.pending.is_none());

        // A tick arriving within MAX_TICK_AGE_SECS of the last check is fresh
        // and clears the streak and the degraded flag (§8.1.3).
        let result = run_tick(&guard, 195, 48_000_000, &tick_data(1, 0)); // 195 - 190 = 5s → fresh
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
        let result = run_tick(&guard, 20, 48_000_000, &tick_data(1, 0));
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
            last_check_ts: 0,
            pending: None,
            pending_ix: None,
            degraded: false,
            stale_streak: 0,
            drift_market_index: 1,
            drift_subaccount_id: 0,
            daily_spent_usd: 0,
            daily_epoch_start_ts: 0,
            venue_size: 0,
            venue_collateral: 0,
            reconcile_ts: 0,
            reconcile_nonce: 0,
            reconcile_status: RECONCILE_NEVER,
            margin_wallet_bump: 0,
        };
        let mut guard_data = vec![0u8; GUARD_DATA_LEN];
        state.write_into(&mut guard_data).unwrap();
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);

        // TakeProfit is the top-priority action (fires before the liquidity
        // gate), so it deterministically reaches the reduce path regardless of
        // solver reachability.
        let result = run_tick(&guard, 20, 49_000_000, &tick_data(1, 0));
        assert_eq!(result, Err(WickError::InvalidInstruction.into()));

        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.nonce, 0);
    }

    #[test]
    fn tick_nonce_fast_forward_rejected() {
        // security: the tick nonce is attacker-controlled (OnPriceTick is
        // permissionless). A nonce far ahead of the committed one would, once
        // committed by a landed action, make every genuine later tick look like
        // a replay — silently disarming the guard. It must step by one.
        let guard_data = make_guard(AuthorityRequirement::CoSigned, 0, 0, 0);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);

        let result = run_tick(&guard, 20, 48_000_000, &tick_data(u64::MAX, 0));
        assert_eq!(result, Err(WickError::NonceOutOfOrder.into()));

        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.nonce, 0);
        assert_eq!(
            state.last_check_ts, 0,
            "a rejected tick must not mutate guard state"
        );

        // The next legal step (nonce + 1) is still accepted.
        assert!(run_tick(&guard, 20, 48_000_000, &tick_data(1, 0)).is_ok());
    }

    #[test]
    fn init_guard_rejects_reinitialization() {
        // security: the runtime only re-derives the PDA from the seeds during
        // the CreateAccount CPI, which an already-funded account skips. Without
        // the initialized check an attacker could pass a victim's guard account
        // with their own key as `owner` and overwrite its state, zeroing the
        // nonce and collateral.
        let victim_owner = addr(9);
        let guard_data = sample_guard(victim_owner);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);

        let attacker = TestAccount::new(
            addr(66),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );
        let payer = TestAccount::new(
            addr(67),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            true,
        );
        let rent = TestAccount::new(
            addr(68),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            false,
            false,
        );

        // [0]=InitGuard, [1]=bump, then the policy blob.
        let mut data = vec![0u8, 0u8];
        data.extend_from_slice(&[0u8; INIT_PAYLOAD_LEN]);
        data[2] = VENUE_DRIFT;
        data[2 + 33] = 0; // Autonomous

        let accounts = [guard.view, attacker.view, payer.view, rent.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &data);
        assert_eq!(result, Err(WickError::AlreadyInitialized.into()));

        // The victim's guard is untouched: owner, collateral and nonce intact.
        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.venue_owner, victim_owner.to_bytes());
        assert_eq!(state.collateral, 100_000_000);
    }

    #[test]
    fn init_route_config_rejects_authority_takeover() {
        // security: InitRouteConfig is permissionless by design (first caller
        // becomes the pause authority), so re-running it against the existing
        // singleton would let anyone install themselves as that authority and
        // clear `paused`. Initialization is one-shot.
        let cfg = RouteConfig {
            authority: [3u8; 32],
            paused: true,
            _padding: [0u8; 31],
        };
        let mut buf = vec![0u8; ROUTE_CONFIG_LEN];
        cfg.write_into(&mut buf).unwrap();
        let config = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &buf, false, true);

        let attacker = TestAccount::new(
            addr(66),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            false,
        );
        let payer = TestAccount::new(
            addr(67),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            true,
        );
        let rent = TestAccount::new(
            addr(68),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            false,
            false,
        );

        let accounts = [config.view, attacker.view, payer.view, rent.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &[10u8, 0u8]);
        assert_eq!(result, Err(WickError::AlreadyInitialized.into()));

        // Authority and the armed kill-switch both survive.
        let back = RouteConfig::from_bytes(config.data()).unwrap();
        assert_eq!(back.authority, [3u8; 32]);
        assert!(back.paused);
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
        let route_config = route_config_account();
        let accounts = [guard.view, bad_clock.view, route_config.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &tick_data(1, 0));
        assert_eq!(result, Err(WickError::InvalidInstruction.into()));
    }

    #[test]
    fn tick_missing_pyth_oracle_rejected() {
        // security: a tick without the authoritative Pyth `PriceUpdateV2`
        // account must be rejected — a cranker cannot fall back to a
        // caller-supplied price (the pre-Pyth behavior was the vuln).
        let guard_data = make_guard(AuthorityRequirement::CoSigned, 0, 0, 0);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        let clock = clock_account(20);
        let route_config = route_config_account();
        // No Pyth account: accounts end after [guard, clock, route_config].
        let accounts = [guard.view, clock.view, route_config.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &tick_data(1, 0));
        assert_eq!(result, Err(WickError::InvalidInstruction.into()));
        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.nonce, 0);
        assert_eq!(state.last_check_ts, 0, "no state mutated by rejected tick");
    }

    #[test]
    fn tick_foreign_oracle_account_rejected() {
        // security: a PriceUpdateV2-shaped account owned by something other
        // than the Pyth receiver program must not price the tick.
        let guard_data = make_guard(AuthorityRequirement::CoSigned, 0, 0, 0);
        let guard = TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &guard_data, false, true);
        let clock = clock_account(20);
        let route_config = route_config_account();
        // Valid layout bytes, but owned by the guard program (attacker on the
        // fixture frame) — the accessor's owner gate must reject.
        let mut data = vec![0u8; 200];
        data[..8].copy_from_slice(&crate::pyth::PRICE_UPDATE_V2_DISCRIMINATOR);
        data[40] = 1;
        data[41..73].copy_from_slice(&crate::pyth::SOL_USD_FEED_ID);
        data[73..81].copy_from_slice(&((49_000_000u128 * 100) as i64).to_le_bytes());
        data[81..89].copy_from_slice(&0u64.to_le_bytes());
        data[89..93].copy_from_slice(&(-8i32).to_le_bytes());
        data[93..101].copy_from_slice(&0i64.to_le_bytes());
        let imposter = TestAccount::new(
            Address::new_from_array([3u8; 32]),
            PROGRAM_ID, // not the Pyth receiver program
            0,
            &data,
            false,
            false,
        );
        let accounts = [guard.view, clock.view, route_config.view, imposter.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &tick_data(1, 99));
        assert_eq!(result, Err(WickError::WrongAccountOwner.into()));
        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.nonce, 0);
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
        let route_config = route_config_account();
        let accounts = [guard.view, owner_acc.view, route_config.view];
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
        let route_config = route_config_account();
        let accounts = [guard.view, stranger.view, route_config.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &data);
        assert_eq!(result, Err(WickError::SignerKeyMismatch.into()));
    }

    // ------------------------------------------------------------------
    //  Post-execution position accounting
    // ------------------------------------------------------------------

    /// A guard that believes it still holds the pre-reduce size re-solves
    /// against the original notional every tick and walks the whole position
    /// out of the venue a slice at a time. The reduce must be reflected.
    #[test]
    fn executed_partial_close_shrinks_size_by_fraction() {
        let mut state = GuardState::from_bytes(&make_guard(
            AuthorityRequirement::Autonomous,
            VENUE_DRIFT,
            0,
            0,
        ))
        .unwrap();
        state.size = 100_000_000;
        apply_executed_reduce(
            &mut state,
            Action::PartialClose {
                fraction_bps: 3_000,
            },
        );
        // Same arithmetic the venue adapter used: 100_000_000 * 3000 / 10_000.
        assert_eq!(state.size, 70_000_000);
    }

    /// Shorts carry a negative size; the reduce shrinks magnitude and must not
    /// flip the sign — a sign flip would have the guard buying to "reduce".
    #[test]
    fn executed_partial_close_preserves_short_sign() {
        let mut state = GuardState::from_bytes(&make_guard(
            AuthorityRequirement::Autonomous,
            VENUE_DRIFT,
            0,
            0,
        ))
        .unwrap();
        state.size = -100_000_000;
        apply_executed_reduce(
            &mut state,
            Action::PartialClose {
                fraction_bps: 2_500,
            },
        );
        assert_eq!(state.size, -75_000_000);
    }

    /// `execute_drift_autonomous` sends `fraction_bps = 10_000` for a
    /// TakeProfit, so the whole watched position leaves the venue.
    #[test]
    fn executed_take_profit_zeroes_size() {
        let mut state = GuardState::from_bytes(&make_guard(
            AuthorityRequirement::Autonomous,
            VENUE_DRIFT,
            0,
            0,
        ))
        .unwrap();
        state.size = -100_000_000;
        apply_executed_reduce(&mut state, Action::TakeProfit);
        assert_eq!(state.size, 0);
    }

    /// The residual must round *up*, never down: the venue floors the reduce to
    /// whole base units, so modelling a smaller residual than the venue holds
    /// would leave real exposure the guard no longer watches.
    #[test]
    fn executed_partial_close_rounds_residual_in_guards_favour() {
        let mut state = GuardState::from_bytes(&make_guard(
            AuthorityRequirement::Autonomous,
            VENUE_DRIFT,
            0,
            0,
        ))
        .unwrap();
        state.size = 7;
        // 7 * 3333 / 10_000 = 2 (floored) — the venue reduced 2, leaving 5.
        apply_executed_reduce(
            &mut state,
            Action::PartialClose {
                fraction_bps: 3_333,
            },
        );
        assert_eq!(state.size, 5);
    }

    /// Neither action can reach the venue (`execute_drift_autonomous` rejects
    /// them), so neither may move the guard's model of the position.
    #[test]
    fn top_up_and_escalate_leave_size_untouched() {
        let mut state = GuardState::from_bytes(&make_guard(
            AuthorityRequirement::Autonomous,
            VENUE_DRIFT,
            0,
            0,
        ))
        .unwrap();
        state.size = 100_000_000;
        apply_executed_reduce(&mut state, Action::TopUp { amount: 1_000 });
        assert_eq!(state.size, 100_000_000);
        apply_executed_reduce(&mut state, Action::EscalateManualReview);
        assert_eq!(state.size, 100_000_000);
    }

    // ------------------------------------------------------------------
    //  CloseGuard
    // ------------------------------------------------------------------

    /// Build a guard account at its real PDA so `close_guard`'s re-derivation
    /// can succeed. Returns the account plus the bump the caller must pass.
    fn guard_at_pda(owner: Address, lamports: u64) -> (TestAccount, u8) {
        let owner_key = owner.to_bytes();
        let (pda, bump) = Address::find_program_address(&[GUARD_SEED, &owner_key], &PROGRAM_ID);
        let data = sample_guard(owner);
        (
            TestAccount::new(pda, PROGRAM_ID, lamports, &data, false, true),
            bump,
        )
    }

    #[test]
    fn close_guard_refunds_rent_and_frees_the_account() {
        let owner = addr(9);
        let (guard, bump) = guard_at_pda(owner, 2_000_000);
        let owner_acc = TestAccount::new(
            owner,
            Address::new_from_array([0u8; 32]),
            500,
            &[],
            true,
            true,
        );

        let accounts = [guard.view, owner_acc.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &[11u8, bump]);
        assert!(result.is_ok());

        // Rent moved to the owner, and the account is emptied so the PDA is
        // free for a fresh `InitGuard`.
        assert_eq!(guard.view.lamports(), 0);
        assert_eq!(owner_acc.view.lamports(), 2_000_500);
        assert_eq!(guard.view.data_len(), 0);
    }

    /// The whole point of the instruction is to recover an account that no
    /// longer decodes, so it must not decode the account to authorize itself.
    #[test]
    fn close_guard_recovers_an_undecodable_guard() {
        let owner = addr(9);
        let owner_key = owner.to_bytes();
        let (pda, bump) = Address::find_program_address(&[GUARD_SEED, &owner_key], &PROGRAM_ID);
        // Right length, garbage version badge — `GuardState::from_bytes` fails.
        let mut junk = vec![0xABu8; GUARD_DATA_LEN];
        junk[0] = 0xFF;
        assert!(GuardState::from_bytes(&junk).is_err());

        let guard = TestAccount::new(pda, PROGRAM_ID, 2_000_000, &junk, false, true);
        let owner_acc = TestAccount::new(
            owner,
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            true,
        );
        let accounts = [guard.view, owner_acc.view];
        assert!(process_instruction(&PROGRAM_ID, &accounts, &[11u8, bump]).is_ok());
        assert_eq!(owner_acc.view.lamports(), 2_000_000);
    }

    /// A stranger's key derives a different PDA, so the re-derivation is what
    /// stops them draining someone else's rent.
    #[test]
    fn close_guard_rejects_a_stranger() {
        let owner = addr(9);
        let (guard, bump) = guard_at_pda(owner, 2_000_000);
        let stranger = TestAccount::new(
            addr(42),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            true,
        );

        let accounts = [guard.view, stranger.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &[11u8, bump]);
        assert_eq!(result, Err(WickError::InvalidPda.into()));
        assert_eq!(guard.view.lamports(), 2_000_000);
    }

    #[test]
    fn close_guard_requires_owner_signature() {
        let owner = addr(9);
        let (guard, bump) = guard_at_pda(owner, 2_000_000);
        // Correct key, but not a signer.
        let owner_acc = TestAccount::new(
            owner,
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            false,
            true,
        );

        let accounts = [guard.view, owner_acc.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &[11u8, bump]);
        assert_eq!(result, Err(WickError::MissingOwnerAuthority.into()));
        assert_eq!(guard.view.lamports(), 2_000_000);
    }

    /// A delegated guard belongs to the Delegation Program. Closing it here
    /// would try to spend from an account we no longer own.
    #[test]
    fn close_guard_rejects_a_guard_we_do_not_own() {
        let owner = addr(9);
        let owner_key = owner.to_bytes();
        let (pda, bump) = Address::find_program_address(&[GUARD_SEED, &owner_key], &PROGRAM_ID);
        let data = sample_guard(owner);
        let guard = TestAccount::new(pda, addr(77), 2_000_000, &data, false, true);
        let owner_acc = TestAccount::new(
            owner,
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            true,
        );

        let accounts = [guard.view, owner_acc.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &[11u8, bump]);
        assert_eq!(result, Err(WickError::WrongAccountOwner.into()));
        assert_eq!(guard.view.lamports(), 2_000_000);
    }

    /// A wrong bump derives a different address (or none at all); either way it
    /// must not authorize a close.
    #[test]
    fn close_guard_rejects_a_wrong_bump() {
        let owner = addr(9);
        let (guard, bump) = guard_at_pda(owner, 2_000_000);
        let owner_acc = TestAccount::new(
            owner,
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            true,
            true,
        );

        let accounts = [guard.view, owner_acc.view];
        let wrong = if bump == 0 { 1 } else { bump - 1 };
        let result = process_instruction(&PROGRAM_ID, &accounts, &[11u8, wrong]);
        assert_eq!(result, Err(WickError::InvalidPda.into()));
        assert_eq!(guard.view.lamports(), 2_000_000);
    }

    // ------------------------------------------------------------------
    //  SetRouteAuthority
    // ------------------------------------------------------------------

    /// A RouteConfig account whose authority is `authority`, optionally paused.
    fn route_config_owned_by(authority: Address, paused: bool) -> TestAccount {
        let cfg = RouteConfig {
            authority: authority.to_bytes(),
            paused,
            _padding: [0u8; 31],
        };
        let mut data = vec![0u8; ROUTE_CONFIG_LEN];
        cfg.write_into(&mut data).unwrap();
        TestAccount::new(
            Address::new_from_array([2u8; 32]),
            PROGRAM_ID,
            0,
            &data,
            false,
            true,
        )
    }

    fn signer_account(key: Address) -> TestAccount {
        TestAccount::new(key, Address::new_from_array([0u8; 32]), 0, &[], true, false)
    }

    #[test]
    fn set_route_authority_rotates_to_the_new_key() {
        let current = addr(1);
        let next = addr(2);
        let config = route_config_owned_by(current, false);
        let cur_acc = signer_account(current);
        let new_acc = signer_account(next);

        let accounts = [config.view, cur_acc.view, new_acc.view];
        assert!(process_instruction(&PROGRAM_ID, &accounts, &[12u8]).is_ok());

        let cfg = RouteConfig::from_bytes(config.data()).unwrap();
        assert_eq!(cfg.authority, next.to_bytes());
    }

    /// Rotating during an incident must not un-pause the program.
    #[test]
    fn set_route_authority_preserves_the_paused_flag() {
        let current = addr(1);
        let next = addr(2);
        let config = route_config_owned_by(current, true);
        let cur_acc = signer_account(current);
        let new_acc = signer_account(next);

        let accounts = [config.view, cur_acc.view, new_acc.view];
        assert!(process_instruction(&PROGRAM_ID, &accounts, &[12u8]).is_ok());

        let cfg = RouteConfig::from_bytes(config.data()).unwrap();
        assert_eq!(cfg.authority, next.to_bytes());
        assert!(cfg.paused);
    }

    #[test]
    fn set_route_authority_rejects_a_stranger() {
        let current = addr(1);
        let config = route_config_owned_by(current, false);
        let impostor = signer_account(addr(42));
        let new_acc = signer_account(addr(2));

        let accounts = [config.view, impostor.view, new_acc.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &[12u8]);
        assert_eq!(result, Err(WickError::Unauthorized.into()));

        let cfg = RouteConfig::from_bytes(config.data()).unwrap();
        assert_eq!(cfg.authority, current.to_bytes());
    }

    /// Rotating a kill switch to an address nobody controls disables it
    /// permanently, and a RouteConfig has no second address to fall back on.
    #[test]
    fn set_route_authority_requires_the_incoming_key_to_sign() {
        let current = addr(1);
        let config = route_config_owned_by(current, false);
        let cur_acc = signer_account(current);
        let unsigned_new = TestAccount::new(
            addr(2),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            false, // does not sign
            false,
        );

        let accounts = [config.view, cur_acc.view, unsigned_new.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &[12u8]);
        assert_eq!(result, Err(WickError::Unauthorized.into()));

        let cfg = RouteConfig::from_bytes(config.data()).unwrap();
        assert_eq!(cfg.authority, current.to_bytes());
    }

    #[test]
    fn set_route_authority_rejects_a_foreign_config_account() {
        let current = addr(1);
        let cfg = RouteConfig {
            authority: current.to_bytes(),
            paused: false,
            _padding: [0u8; 31],
        };
        let mut data = vec![0u8; ROUTE_CONFIG_LEN];
        cfg.write_into(&mut data).unwrap();
        // Owned by someone else — an attacker-supplied lookalike.
        let config = TestAccount::new(
            Address::new_from_array([2u8; 32]),
            addr(77),
            0,
            &data,
            false,
            true,
        );
        let cur_acc = signer_account(current);
        let new_acc = signer_account(addr(2));

        let accounts = [config.view, cur_acc.view, new_acc.view];
        let result = process_instruction(&PROGRAM_ID, &accounts, &[12u8]);
        assert_eq!(result, Err(WickError::WrongAccountOwner.into()));
    }

    /// The rotated-in key can pause; the rotated-out key can no longer.
    #[test]
    fn rotated_authority_controls_the_kill_switch() {
        let current = addr(1);
        let next = addr(2);
        let config = route_config_owned_by(current, false);
        let cur_acc = signer_account(current);
        let new_acc = signer_account(next);

        let accounts = [config.view, cur_acc.view, new_acc.view];
        assert!(process_instruction(&PROGRAM_ID, &accounts, &[12u8]).is_ok());

        // Old authority is now powerless.
        let accounts = [config.view, cur_acc.view];
        assert_eq!(
            process_instruction(&PROGRAM_ID, &accounts, &[3u8, 1]),
            Err(WickError::Unauthorized.into())
        );

        // New authority can pause.
        let accounts = [config.view, new_acc.view];
        assert!(process_instruction(&PROGRAM_ID, &accounts, &[3u8, 1]).is_ok());
        assert!(RouteConfig::from_bytes(config.data()).unwrap().paused);
    }

    // ------------------------------------------------------------------
    //  §8.3 Venue reconciliation
    // ------------------------------------------------------------------

    use crate::account::RECONCILE_CONVERGED;
    use crate::drift::{synthetic_user, DRIFT_PROGRAM_ID};

    const VENUE_OWNER: [u8; 32] = [9u8; 32];

    /// An autonomous Drift guard modelling a 0.1-unit long on market 0.
    fn drift_guard_state() -> GuardState {
        GuardState {
            venue: VENUE_DRIFT,
            venue_owner: VENUE_OWNER,
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
                take_profit: None,
            },
            collateral: 100_000_000,
            size: 100_000_000,
            entry: 50_000_000,
            current_price: 50_000_000,
            nonce: 0,
            last_check_ts: 0,
            pending: None,
            pending_ix: None,
            degraded: false,
            stale_streak: 0,
            drift_market_index: 0,
            drift_subaccount_id: 0,
            daily_spent_usd: 0,
            daily_epoch_start_ts: 0,
            venue_size: 0,
            venue_collateral: 0,
            reconcile_ts: 0,
            reconcile_nonce: 0,
            reconcile_status: RECONCILE_NEVER,
            margin_wallet_bump: 0,
        }
    }

    fn guard_account_for(state: &GuardState) -> TestAccount {
        let mut buf = vec![0u8; GUARD_DATA_LEN];
        state.write_into(&mut buf).unwrap();
        TestAccount::new(PROGRAM_ID, PROGRAM_ID, 100, &buf, false, true)
    }

    /// A Velocity `User` account at the PDA the guard re-derives, owned by the
    /// venue program — i.e. the only account `verify_user_account` will accept.
    fn venue_user_account(
        subaccount: u16,
        market_index: u16,
        base: i64,
        scaled: u64,
    ) -> TestAccount {
        let (pda, _bump) = Address::find_program_address(
            &[b"user", &VENUE_OWNER, &subaccount.to_le_bytes()],
            &DRIFT_PROGRAM_ID,
        );
        let data = synthetic_user(market_index, base, scaled);
        TestAccount::new(pda, DRIFT_PROGRAM_ID, 0, &data, false, false)
    }

    /// `ReconcileVenue` payload: [13, nonce:8].
    fn reconcile_data(nonce: u64) -> [u8; 9] {
        let mut d = [0u8; 9];
        d[0] = 13;
        d[1..9].copy_from_slice(&nonce.to_le_bytes());
        d
    }

    fn run_reconcile(
        guard: &TestAccount,
        venue: &TestAccount,
        ts: i64,
        nonce: u64,
    ) -> Result<(), pinocchio::error::ProgramError> {
        let clock = clock_account(ts);
        let route_config = route_config_account();
        let accounts = [guard.view, clock.view, route_config.view, venue.view];
        process_instruction(&PROGRAM_ID, &accounts, &reconcile_data(nonce))
    }

    #[test]
    fn reconcile_records_the_venues_own_numbers() {
        let guard = guard_account_for(&drift_guard_state());
        // Venue agrees exactly; 1e15 scaled balance = 1e12 in the guard's 6dp.
        let venue = venue_user_account(0, 0, 100_000_000, 1_000_000_000_000_000);

        assert!(run_reconcile(&guard, &venue, 1_700_000_000, 1).is_ok());

        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.reconcile_status, RECONCILE_CONVERGED);
        assert_eq!(state.venue_size, 100_000_000);
        assert_eq!(state.venue_collateral, 1_000_000_000_000);
        assert_eq!(state.reconcile_ts, 1_700_000_000);
        assert_eq!(state.reconcile_nonce, 1);
        // Reconciliation observes; it never edits the model or moves value.
        assert_eq!(state.size, 100_000_000);
        assert_eq!(state.collateral, 100_000_000);
        assert!(state.pending.is_none());
    }

    /// Inside `RECONCILE_TOLERANCE_BPS` (25 bps) the guard stays armed — funding
    /// and fill dust must not disarm it.
    #[test]
    fn reconcile_absorbs_dust_sized_disagreement() {
        let guard = guard_account_for(&drift_guard_state());
        // 10 bps under the model: 100_000_000 → 99_900_000.
        let venue = venue_user_account(0, 0, 99_900_000, 0);
        assert!(run_reconcile(&guard, &venue, 1_700_000_000, 1).is_ok());
        assert_eq!(
            GuardState::from_bytes(guard.data())
                .unwrap()
                .reconcile_status,
            RECONCILE_CONVERGED
        );
    }

    /// The load-bearing test for §8.3: a divergence must be **recorded**, not
    /// raised. Returning an error would roll back the very write that disarms
    /// the guard, leaving it armed on a model the venue has contradicted.
    #[test]
    fn reconcile_records_divergence_rather_than_raising_it() {
        let guard = guard_account_for(&drift_guard_state());
        let venue = venue_user_account(0, 0, 50_000_000, 0); // half the model

        let result = run_reconcile(&guard, &venue, 1_700_000_000, 1);
        assert!(result.is_ok(), "a diverged verdict must commit, not error");

        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.reconcile_status, RECONCILE_DIVERGED);
        assert_eq!(state.venue_size, 50_000_000);
    }

    /// The owner closed at the venue behind the guard's back. Flat-vs-exposed
    /// has no ratio to test, so it is divergence by construction.
    #[test]
    fn reconcile_notices_a_position_closed_at_the_venue() {
        let guard = guard_account_for(&drift_guard_state());
        let venue = venue_user_account(0, 0, 0, 0);
        assert!(run_reconcile(&guard, &venue, 1_700_000_000, 1).is_ok());

        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.reconcile_status, RECONCILE_DIVERGED);
        assert_eq!(state.venue_size, 0);
    }

    /// `ReconcileVenue` is permissionless, so a replayed transaction is free to
    /// send. Without a strictly-increasing nonce it would re-apply an old
    /// snapshot over a newer one — including re-arming a guard that has since
    /// diverged.
    #[test]
    fn reconcile_rejects_a_replayed_or_stale_nonce() {
        let guard = guard_account_for(&drift_guard_state());
        let venue = venue_user_account(0, 0, 100_000_000, 0);

        assert!(run_reconcile(&guard, &venue, 1_700_000_000, 5).is_ok());
        // Same nonce again — the literal replay.
        assert_eq!(
            run_reconcile(&guard, &venue, 1_700_000_100, 5),
            Err(WickError::ReconcileStale.into())
        );
        // And anything behind it.
        assert_eq!(
            run_reconcile(&guard, &venue, 1_700_000_100, 4),
            Err(WickError::ReconcileStale.into())
        );
        // The stored observation is the one from nonce 5, untouched.
        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.reconcile_nonce, 5);
        assert_eq!(state.reconcile_ts, 1_700_000_000);
    }

    /// security: the position account is attacker-supplied. Bytes that decode
    /// are not enough — the account must be *owned by the venue program*, or a
    /// caller could hand the guard a look-alike they wrote themselves and have
    /// a fabricated position adopted as ground truth.
    #[test]
    fn reconcile_rejects_a_venue_account_the_venue_does_not_own() {
        let guard = guard_account_for(&drift_guard_state());
        let (pda, _) = Address::find_program_address(
            &[b"user", &VENUE_OWNER, &0u16.to_le_bytes()],
            &DRIFT_PROGRAM_ID,
        );
        // Right address, right bytes — wrong owner.
        let data = synthetic_user(0, 1, 0);
        let forged = TestAccount::new(pda, addr(66), 0, &data, false, false);

        assert_eq!(
            run_reconcile(&guard, &forged, 1_700_000_000, 1),
            Err(WickError::VenueAccountMismatch.into())
        );
        let state = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(state.reconcile_status, RECONCILE_NEVER);
        assert_eq!(state.reconcile_nonce, 0);
    }

    /// The other half of the check: a genuine venue account belonging to a
    /// *different* sub-account than the one this guard watches.
    #[test]
    fn reconcile_rejects_a_different_subaccount() {
        let guard = guard_account_for(&drift_guard_state()); // watches sub 0
        let venue = venue_user_account(7, 0, 100_000_000, 0);
        assert_eq!(
            run_reconcile(&guard, &venue, 1_700_000_000, 1),
            Err(WickError::VenueAccountMismatch.into())
        );
    }

    /// A guard on a co-signed venue has no position account the program can
    /// decode. Admitting that beats guessing at one.
    #[test]
    fn reconcile_rejects_a_non_drift_venue() {
        let mut state = drift_guard_state();
        state.venue = VENUE_JUPITER;
        let guard = guard_account_for(&state);
        let venue = venue_user_account(0, 0, 100_000_000, 0);
        assert_eq!(
            run_reconcile(&guard, &venue, 1_700_000_000, 1),
            Err(WickError::UnsupportedVenueAction.into())
        );
    }

    #[test]
    fn reconcile_requires_the_venue_account_and_an_exact_payload() {
        let guard = guard_account_for(&drift_guard_state());
        let clock = clock_account(1_700_000_000);
        let route_config = route_config_account();

        // Venue account omitted entirely.
        let short = [guard.view, clock.view, route_config.view];
        assert_eq!(
            process_instruction(&PROGRAM_ID, &short, &reconcile_data(1)),
            Err(WickError::InvalidInstruction.into())
        );

        // Truncated nonce.
        let venue = venue_user_account(0, 0, 100_000_000, 0);
        let accounts = [guard.view, clock.view, route_config.view, venue.view];
        assert_eq!(
            process_instruction(&PROGRAM_ID, &accounts, &[13u8, 1, 0, 0]),
            Err(WickError::InvalidInstruction.into())
        );
        // Trailing junk is rejected too — a payload the program does not fully
        // understand is a payload it must not act on.
        assert_eq!(
            process_instruction(&PROGRAM_ID, &accounts, &[13u8, 1, 0, 0, 0, 0, 0, 0, 0, 0]),
            Err(WickError::InvalidInstruction.into())
        );
    }

    #[test]
    fn reconcile_is_refused_while_paused() {
        let guard = guard_account_for(&drift_guard_state());
        let venue = venue_user_account(0, 0, 100_000_000, 0);
        let clock = clock_account(1_700_000_000);
        let paused = route_config_owned_by(addr(1), true);
        let accounts = [guard.view, clock.view, paused.view, venue.view];
        assert_eq!(
            process_instruction(&PROGRAM_ID, &accounts, &reconcile_data(1)),
            Err(WickError::Paused.into())
        );
    }

    // ------------------------------------------------------------------
    //  §8.3.4 A diverged model disarms autonomous execution
    // ------------------------------------------------------------------

    /// Every autonomous order is sized *from* `state.size`. Once the venue has
    /// contradicted that number the guard must stop trading it: too small and
    /// the breach is not cleared, too large and a reduce that should trim the
    /// position closes it outright.
    ///
    /// The distinguishing signal is the error, not the pending action. With no
    /// venue adapter accounts supplied, a *converged* guard reaches
    /// `execute_autonomous` and fails hard (`InvalidInstruction`, as
    /// `tick_drift_missing_venue_accounts_rejected` pins); a diverged one never
    /// gets there and returns `Ok` with an escalation. Same fixture, opposite
    /// outcome — so this test cannot pass for the wrong reason.
    #[test]
    fn diverged_guard_escalates_instead_of_executing() {
        let mut state = drift_guard_state();
        state.reconcile_status = RECONCILE_DIVERGED;
        state.policy.take_profit = Some(49_000_000);
        let guard = guard_account_for(&state);

        let result = run_tick(&guard, 20, 49_000_000, &tick_data(1, 0));
        assert!(
            result.is_ok(),
            "diverged tick must not fail the transaction"
        );

        let after = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(after.pending, Some(Action::EscalateManualReview));
        // Nonce uncommitted, so the guard re-arms the moment the model is fixed.
        assert_eq!(after.nonce, 0);
        assert_eq!(after.daily_spent_usd, 0);
        // Health still updates — a disarmed guard is not a blind one.
        assert_eq!(after.current_price, 49_000_000);
        assert_eq!(after.last_check_ts, 20);
    }

    #[test]
    fn converged_guard_still_reaches_the_venue_adapter() {
        let mut state = drift_guard_state();
        state.reconcile_status = RECONCILE_CONVERGED;
        state.policy.take_profit = Some(49_000_000);
        let guard = guard_account_for(&state);

        // Same missing venue accounts as above, but the guard is armed: the
        // adapter is reached and rejects. This is the control for the test above.
        assert_eq!(
            run_tick(&guard, 20, 49_000_000, &tick_data(1, 0)),
            Err(WickError::InvalidInstruction.into())
        );
    }

    /// An owner `UpdatePosition` is the documented way out of a diverged freeze:
    /// it resets the verdict to `NeverReconciled` (never to `Converged` — the
    /// owner asserting a number is not the venue agreeing to it).
    #[test]
    fn update_position_clears_a_diverged_freeze() {
        let mut state = drift_guard_state();
        state.reconcile_status = RECONCILE_DIVERGED;
        state.venue_size = 50_000_000;
        state.reconcile_ts = 1_700_000_000;
        let guard = guard_account_for(&state);

        let mut data = vec![8u8];
        data.extend_from_slice(&100_000_000u128.to_le_bytes());
        data.extend_from_slice(&50_000_000i128.to_le_bytes());
        data.extend_from_slice(&50_000_000u128.to_le_bytes());
        let owner_acc = signer_account(Address::from(VENUE_OWNER));
        let route_config = route_config_account();
        let accounts = [guard.view, owner_acc.view, route_config.view];
        assert!(process_instruction(&PROGRAM_ID, &accounts, &data).is_ok());

        let after = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(after.reconcile_status, RECONCILE_NEVER);
        assert_eq!(after.size, 50_000_000);
        // The observation itself survives — it is a timestamped fact, and the
        // console renders it beside the stamp.
        assert_eq!(after.venue_size, 50_000_000);
        assert_eq!(after.reconcile_ts, 1_700_000_000);
    }

    // ------------------------------------------------------------------
    //  §8.5 Margin wallet — a real, rent-backed, 2-of-2 lamport reserve
    // ------------------------------------------------------------------
    //
    //  A note on what these tests can and cannot prove. Off-target, a CPI is a
    //  no-op that returns `Ok` (`solana-instruction-view`'s
    //  `invoke_signed_unchecked` compiles to a `black_box` when the target is
    //  not Solana), so `CreateAccount` and `Transfer` move nothing here. These
    //  tests therefore cover the parts that are this program's own logic —
    //  derivation, authority, accounting, and the rent-backing invariant — with
    //  the fixture standing in for the lamports the runtime would have moved.
    //  `tests/margin_wallet.rs` runs the same flow against the real SBF VM,
    //  where the transfers actually happen.

    use pinocchio::sysvars::rent::{DEFAULT_LAMPORTS_PER_BYTE, RENT_ID};

    /// `f64::to_le_bytes` of `2.0` — the current exemption threshold. Pinned as
    /// bytes because the on-chain `Rent` avoids floating-point entirely.
    const EXEMPTION_THRESHOLD_2_0: [u8; 8] = [0, 0, 0, 0, 0, 0, 0, 64];

    /// The Rent sysvar. Address must be `RENT_ID` or `Rent::from_account_view`
    /// rejects it. Layout: `lamports_per_byte` (u64) at 0, `exemption_threshold`
    /// (f64 LE bytes) at 8.
    fn rent_account() -> TestAccount {
        let mut data = vec![0u8; 17];
        data[0..8].copy_from_slice(&DEFAULT_LAMPORTS_PER_BYTE.to_le_bytes());
        data[8..16].copy_from_slice(&EXEMPTION_THRESHOLD_2_0);
        data[16] = 50; // burn_percent — present on the real sysvar, unread here
        TestAccount::new(
            RENT_ID,
            Address::new_from_array([0u8; 32]),
            0,
            &data,
            false,
            false,
        )
    }

    /// What the runtime would demand to keep an 81-byte account alive. Computed
    /// from the same sysvar the program reads rather than restated as a literal.
    fn wallet_rent_minimum() -> u64 {
        2 * (128 + WALLET_DATA_LEN as u64) * DEFAULT_LAMPORTS_PER_BYTE
    }

    fn margin_pda() -> (Address, u8) {
        Address::find_program_address(&[MARGIN_WALLET_SEED, &VENUE_OWNER], &PROGRAM_ID)
    }

    /// A margin wallet PDA holding `balance` lamports on the owner's behalf,
    /// funded to exactly the invariant: rent minimum + balance.
    fn wallet_account(balance: u128, extra_lamports: u64) -> TestAccount {
        let (pda, _) = margin_pda();
        let ws = WalletState {
            owner: VENUE_OWNER,
            co_authority: [5u8; 32],
            balance,
        };
        let mut data = vec![0u8; WALLET_DATA_LEN];
        ws.write_into(&mut data).unwrap();
        let lamports = wallet_rent_minimum() + balance as u64 + extra_lamports;
        TestAccount::new(pda, PROGRAM_ID, lamports, &data, false, true)
    }

    /// An uninitialized wallet at the right PDA: program-owned and correctly
    /// sized (what `CreateAccount` leaves behind on-chain), zero lamports so
    /// `init_margin_wallet` takes the create branch.
    fn uninit_wallet_account() -> TestAccount {
        let (pda, _) = margin_pda();
        let data = vec![0u8; WALLET_DATA_LEN];
        TestAccount::new(pda, PROGRAM_ID, 0, &data, false, true)
    }

    fn writable_signer(key: Address, lamports: u64) -> TestAccount {
        TestAccount::new(
            key,
            Address::new_from_array([0u8; 32]),
            lamports,
            &[],
            true,
            true,
        )
    }

    fn amount_data(disc: u8, amount: u128) -> Vec<u8> {
        let mut d = vec![disc];
        d.extend_from_slice(&amount.to_le_bytes());
        d
    }

    #[test]
    fn init_margin_wallet_links_the_reserve_to_the_guard() {
        let (_, bump) = margin_pda();
        let guard = guard_account_for(&drift_guard_state());
        let wallet = uninit_wallet_account();
        let owner = writable_signer(Address::from(VENUE_OWNER), 10 * wallet_rent_minimum());
        let rent = rent_account();
        let route_config = route_config_account();

        let accounts = [
            wallet.view,
            guard.view,
            owner.view,
            owner.view,
            rent.view,
            route_config.view,
        ];
        assert!(process_instruction(&PROGRAM_ID, &accounts, &[14u8, bump]).is_ok());

        // The guard now knows how to re-derive its reserve.
        assert_eq!(
            GuardState::from_bytes(guard.data())
                .unwrap()
                .margin_wallet_bump,
            bump
        );
        // The wallet inherits the guard's own 2-of-2 pair, so the exit path
        // cannot be widened by pointing a wallet at a friendlier co-authority.
        let ws = WalletState::from_bytes(wallet.data()).unwrap();
        assert_eq!(ws.owner, VENUE_OWNER);
        assert_eq!(ws.co_authority, [5u8; 32]);
        assert_eq!(ws.balance, 0);
    }

    /// Same trap `init_guard` guards against: the runtime only re-derives a PDA
    /// during `CreateAccount`, which a funded account skips. Re-initializing a
    /// live wallet would zero a `balance` the owner funded.
    #[test]
    fn init_margin_wallet_rejects_reinitialization() {
        let (_, bump) = margin_pda();
        let guard = guard_account_for(&drift_guard_state());
        let wallet = wallet_account(5_000_000, 0); // already live and funded
        let owner = writable_signer(Address::from(VENUE_OWNER), 10 * wallet_rent_minimum());
        let rent = rent_account();
        let route_config = route_config_account();

        let accounts = [
            wallet.view,
            guard.view,
            owner.view,
            owner.view,
            rent.view,
            route_config.view,
        ];
        assert_eq!(
            process_instruction(&PROGRAM_ID, &accounts, &[14u8, bump]),
            Err(WickError::AlreadyInitialized.into())
        );
        // The funded balance survives the attempt.
        assert_eq!(
            WalletState::from_bytes(wallet.data()).unwrap().balance,
            5_000_000
        );
    }

    #[test]
    fn init_margin_wallet_requires_the_guards_own_owner() {
        let (_, bump) = margin_pda();
        let guard = guard_account_for(&drift_guard_state());
        let wallet = uninit_wallet_account();
        let stranger = writable_signer(addr(77), 10 * wallet_rent_minimum());
        let rent = rent_account();
        let route_config = route_config_account();

        let accounts = [
            wallet.view,
            guard.view,
            stranger.view,
            stranger.view,
            rent.view,
            route_config.view,
        ];
        assert_eq!(
            process_instruction(&PROGRAM_ID, &accounts, &[14u8, bump]),
            Err(WickError::SignerKeyMismatch.into())
        );
        assert_eq!(
            GuardState::from_bytes(guard.data())
                .unwrap()
                .margin_wallet_bump,
            0
        );
    }

    /// Fixture note: the wallet is pre-funded to the post-transfer balance,
    /// because the System CPI is a host no-op (see the section comment). What
    /// this test pins is the accounting and the invariant check that follows it.
    #[test]
    fn fund_margin_wallet_credits_the_recorded_balance() {
        let (_, bump) = margin_pda();
        let mut gs = drift_guard_state();
        gs.margin_wallet_bump = bump;
        let guard = guard_account_for(&gs);
        // Already holds 1_000_000; the 2_000_000 being funded is present too.
        let wallet = wallet_account(1_000_000, 2_000_000);
        let owner = writable_signer(Address::from(VENUE_OWNER), 10 * wallet_rent_minimum());
        let rent = rent_account();
        let route_config = route_config_account();

        let accounts = [
            wallet.view,
            guard.view,
            owner.view,
            rent.view,
            route_config.view,
        ];
        assert!(process_instruction(&PROGRAM_ID, &accounts, &amount_data(15, 2_000_000)).is_ok());

        assert_eq!(
            WalletState::from_bytes(wallet.data()).unwrap().balance,
            3_000_000
        );
    }

    /// The invariant is the whole point of §8.5: without it `balance` is just a
    /// number again. Here the lamports never arrive (the CPI is a host no-op and
    /// nothing pre-funds them), and the guard refuses to record value it cannot
    /// see rather than book a credit against a transfer that did not land.
    #[test]
    fn fund_margin_wallet_refuses_an_unbacked_credit() {
        let (_, bump) = margin_pda();
        let mut gs = drift_guard_state();
        gs.margin_wallet_bump = bump;
        let guard = guard_account_for(&gs);
        let wallet = wallet_account(0, 0); // exactly rent — no room for a credit
        let owner = writable_signer(Address::from(VENUE_OWNER), 10 * wallet_rent_minimum());
        let rent = rent_account();
        let route_config = route_config_account();

        let accounts = [
            wallet.view,
            guard.view,
            owner.view,
            rent.view,
            route_config.view,
        ];
        assert_eq!(
            process_instruction(&PROGRAM_ID, &accounts, &amount_data(15, 5_000_000)),
            Err(WickError::InsufficientMarginWallet.into())
        );
    }

    /// security: accepting a foreign wallet would let a guard credit itself from
    /// value its owner does not control — or drain somebody else's reserve.
    #[test]
    fn fund_margin_wallet_rejects_a_wallet_it_cannot_re_derive() {
        let (_, bump) = margin_pda();
        let mut gs = drift_guard_state();
        gs.margin_wallet_bump = bump;
        let guard = guard_account_for(&gs);

        // A well-formed, program-owned wallet — belonging to a different owner.
        let (foreign_pda, _) =
            Address::find_program_address(&[MARGIN_WALLET_SEED, &[3u8; 32]], &PROGRAM_ID);
        let ws = WalletState {
            owner: [3u8; 32],
            co_authority: [5u8; 32],
            balance: 9_000_000,
        };
        let mut data = vec![0u8; WALLET_DATA_LEN];
        ws.write_into(&mut data).unwrap();
        let foreign = TestAccount::new(foreign_pda, PROGRAM_ID, 50_000_000, &data, false, true);
        let owner = writable_signer(Address::from(VENUE_OWNER), 10 * wallet_rent_minimum());
        let rent = rent_account();
        let route_config = route_config_account();

        let accounts = [
            foreign.view,
            guard.view,
            owner.view,
            rent.view,
            route_config.view,
        ];
        assert_eq!(
            process_instruction(&PROGRAM_ID, &accounts, &amount_data(15, 1_000_000)),
            Err(WickError::MarginWalletMismatch.into())
        );
        assert_eq!(
            WalletState::from_bytes(foreign.data()).unwrap().balance,
            9_000_000
        );
    }

    /// A guard with no reserve linked has nothing to fund. Silently accepting
    /// would write a wallet the guard cannot find again.
    #[test]
    fn fund_margin_wallet_rejects_an_unlinked_guard() {
        let guard = guard_account_for(&drift_guard_state()); // bump == 0
        let wallet = wallet_account(0, 5_000_000);
        let owner = writable_signer(Address::from(VENUE_OWNER), 10 * wallet_rent_minimum());
        let rent = rent_account();
        let route_config = route_config_account();

        let accounts = [
            wallet.view,
            guard.view,
            owner.view,
            rent.view,
            route_config.view,
        ];
        assert_eq!(
            process_instruction(&PROGRAM_ID, &accounts, &amount_data(15, 1_000_000)),
            Err(WickError::MarginWalletMismatch.into())
        );
    }

    /// The withdraw path moves lamports by direct mutation rather than a System
    /// CPI (System will not debit an account this program owns), so unlike
    /// funding it is fully exercised here — real lamports, real balances.
    fn withdraw_accounts(
        wallet: &TestAccount,
        guard: &TestAccount,
        owner: &TestAccount,
        co: &TestAccount,
        rent: &TestAccount,
        route_config: &TestAccount,
    ) -> [AccountView; 6] {
        [
            wallet.view,
            guard.view,
            owner.view,
            co.view,
            rent.view,
            route_config.view,
        ]
    }

    #[test]
    fn withdraw_margin_wallet_moves_lamports_on_two_signatures() {
        let (_, bump) = margin_pda();
        let mut gs = drift_guard_state();
        gs.margin_wallet_bump = bump;
        let guard = guard_account_for(&gs);
        let wallet = wallet_account(5_000_000, 0);
        let owner = writable_signer(Address::from(VENUE_OWNER), 1_000);
        let co = signer_account(addr(5));
        let rent = rent_account();
        let route_config = route_config_account();

        let wallet_before = wallet.view.lamports();
        let accounts = withdraw_accounts(&wallet, &guard, &owner, &co, &rent, &route_config);
        assert!(process_instruction(&PROGRAM_ID, &accounts, &amount_data(16, 2_000_000)).is_ok());

        assert_eq!(
            WalletState::from_bytes(wallet.data()).unwrap().balance,
            3_000_000
        );
        assert_eq!(wallet.view.lamports(), wallet_before - 2_000_000);
        assert_eq!(owner.view.lamports(), 1_000 + 2_000_000);
    }

    /// §8.5 — value only leaves on two signatures. Either one alone is refused,
    /// and a correct key with the signer flag unset is not a signature.
    #[test]
    fn withdraw_margin_wallet_requires_both_signatures() {
        let (_, bump) = margin_pda();
        let mut gs = drift_guard_state();
        gs.margin_wallet_bump = bump;
        let guard = guard_account_for(&gs);
        let wallet = wallet_account(5_000_000, 0);
        let rent = rent_account();
        let route_config = route_config_account();
        let owner = writable_signer(Address::from(VENUE_OWNER), 0);
        let unsigned_co = TestAccount::new(
            addr(5),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            false,
            false,
        );

        let accounts =
            withdraw_accounts(&wallet, &guard, &owner, &unsigned_co, &rent, &route_config);
        assert_eq!(
            process_instruction(&PROGRAM_ID, &accounts, &amount_data(16, 1)),
            Err(WickError::MissingCoAuthority.into())
        );

        // Co-authority alone.
        let unsigned_owner = TestAccount::new(
            Address::from(VENUE_OWNER),
            Address::new_from_array([0u8; 32]),
            0,
            &[],
            false,
            true,
        );
        let co = signer_account(addr(5));
        let accounts =
            withdraw_accounts(&wallet, &guard, &unsigned_owner, &co, &rent, &route_config);
        assert_eq!(
            process_instruction(&PROGRAM_ID, &accounts, &amount_data(16, 1)),
            Err(WickError::MissingOwnerAuthority.into())
        );

        // A stranger holding a real signature is still a stranger.
        let stranger = writable_signer(addr(88), 0);
        let accounts = withdraw_accounts(&wallet, &guard, &stranger, &co, &rent, &route_config);
        assert_eq!(
            process_instruction(&PROGRAM_ID, &accounts, &amount_data(16, 1)),
            Err(WickError::MissingOwnerAuthority.into())
        );

        // Nothing moved on any of the three attempts.
        assert_eq!(
            WalletState::from_bytes(wallet.data()).unwrap().balance,
            5_000_000
        );
        assert_eq!(wallet.view.lamports(), wallet_rent_minimum() + 5_000_000);
    }

    #[test]
    fn withdraw_margin_wallet_rejects_more_than_the_balance() {
        let (_, bump) = margin_pda();
        let mut gs = drift_guard_state();
        gs.margin_wallet_bump = bump;
        let guard = guard_account_for(&gs);
        // Extra lamports sitting in the account are *not* withdrawable: they are
        // not credited to the owner, and treating them as spendable would let a
        // stray airdrop become someone's balance.
        let wallet = wallet_account(1_000_000, 9_000_000);
        let owner = writable_signer(Address::from(VENUE_OWNER), 0);
        let co = signer_account(addr(5));
        let rent = rent_account();
        let route_config = route_config_account();

        let accounts = withdraw_accounts(&wallet, &guard, &owner, &co, &rent, &route_config);
        assert_eq!(
            process_instruction(&PROGRAM_ID, &accounts, &amount_data(16, 1_000_001)),
            Err(WickError::InsufficientMarginWallet.into())
        );
        assert_eq!(owner.view.lamports(), 0);
    }

    /// Draining the whole balance is legal and must leave the account rent-alive:
    /// if rent did not survive, the runtime could reap the account and any future
    /// `balance` with it.
    #[test]
    fn withdraw_margin_wallet_drains_to_exactly_rent() {
        let (_, bump) = margin_pda();
        let mut gs = drift_guard_state();
        gs.margin_wallet_bump = bump;
        let guard = guard_account_for(&gs);
        let wallet = wallet_account(4_000_000, 0);
        let owner = writable_signer(Address::from(VENUE_OWNER), 0);
        let co = signer_account(addr(5));
        let rent = rent_account();
        let route_config = route_config_account();

        let accounts = withdraw_accounts(&wallet, &guard, &owner, &co, &rent, &route_config);
        assert!(process_instruction(&PROGRAM_ID, &accounts, &amount_data(16, 4_000_000)).is_ok());

        assert_eq!(WalletState::from_bytes(wallet.data()).unwrap().balance, 0);
        assert_eq!(wallet.view.lamports(), wallet_rent_minimum());
        assert_eq!(owner.view.lamports(), 4_000_000);
    }

    /// A wallet whose recorded pair no longer matches the guard's is not this
    /// guard's reserve, even if it re-derives: the 2-of-2 the value was placed
    /// under must be the 2-of-2 that releases it.
    #[test]
    fn withdraw_margin_wallet_rejects_a_wallet_with_a_foreign_pair() {
        let (pda, bump) = margin_pda();
        let mut gs = drift_guard_state();
        gs.margin_wallet_bump = bump;
        let guard = guard_account_for(&gs);

        let ws = WalletState {
            owner: VENUE_OWNER,
            co_authority: [1u8; 32], // not the guard's co_authority
            balance: 5_000_000,
        };
        let mut data = vec![0u8; WALLET_DATA_LEN];
        ws.write_into(&mut data).unwrap();
        let wallet = TestAccount::new(
            pda,
            PROGRAM_ID,
            wallet_rent_minimum() + 5_000_000,
            &data,
            false,
            true,
        );
        let owner = writable_signer(Address::from(VENUE_OWNER), 0);
        let co = signer_account(addr(5));
        let rent = rent_account();
        let route_config = route_config_account();

        let accounts = withdraw_accounts(&wallet, &guard, &owner, &co, &rent, &route_config);
        assert_eq!(
            process_instruction(&PROGRAM_ID, &accounts, &amount_data(16, 1_000_000)),
            Err(WickError::MarginWalletMismatch.into())
        );
        assert_eq!(owner.view.lamports(), 0);
    }

    #[test]
    fn margin_wallet_instructions_are_refused_while_paused() {
        let (_, bump) = margin_pda();
        let mut gs = drift_guard_state();
        gs.margin_wallet_bump = bump;
        let guard = guard_account_for(&gs);
        let wallet = wallet_account(5_000_000, 0);
        let owner = writable_signer(Address::from(VENUE_OWNER), 0);
        let co = signer_account(addr(5));
        let rent = rent_account();
        let paused = route_config_owned_by(addr(1), true);

        let fund = [wallet.view, guard.view, owner.view, rent.view, paused.view];
        assert_eq!(
            process_instruction(&PROGRAM_ID, &fund, &amount_data(15, 1)),
            Err(WickError::Paused.into())
        );

        let withdraw = withdraw_accounts(&wallet, &guard, &owner, &co, &rent, &paused);
        assert_eq!(
            process_instruction(&PROGRAM_ID, &withdraw, &amount_data(16, 1)),
            Err(WickError::Paused.into())
        );
    }

    /// Lamport amounts are carried as u128 on the wire for consistency with the
    /// USD fields, but a lamport count is a u64 on-chain. An amount that cannot
    /// narrow is rejected rather than wrapped into a different number.
    #[test]
    fn margin_wallet_rejects_an_unrepresentable_lamport_amount() {
        let (_, bump) = margin_pda();
        let mut gs = drift_guard_state();
        gs.margin_wallet_bump = bump;
        let guard = guard_account_for(&gs);
        let wallet = wallet_account(0, 0);
        let owner = writable_signer(Address::from(VENUE_OWNER), 0);
        let rent = rent_account();
        let route_config = route_config_account();

        let accounts = [
            wallet.view,
            guard.view,
            owner.view,
            rent.view,
            route_config.view,
        ];
        assert_eq!(
            process_instruction(
                &PROGRAM_ID,
                &accounts,
                &amount_data(15, u128::from(u64::MAX) + 1)
            ),
            Err(WickError::MathOverflow.into())
        );
    }

    /// An autonomous top-up has no Drift adapter behind it (a deposit needs an
    /// SPL transfer into the quote spot market against the vault and oracle —
    /// accounts a tick does not carry), so it escalates to the owner instead of
    /// crediting collateral the venue never received.
    ///
    /// Honest scope: the `margin_wallet_bump == 0` gate in `on_price_tick` and
    /// the adapter's own refusal converge on the same outcome today, so this
    /// pins the observable property — a top-up never silently no-ops, never
    /// credits collateral, and never advances the nonce — in both reserve
    /// states, rather than claiming to distinguish the two arms.
    #[test]
    fn autonomous_top_up_escalates_and_commits_nothing() {
        let (_, bump) = margin_pda();
        for reserve_bump in [0, bump] {
            let mut gs = drift_guard_state();
            gs.margin_wallet_bump = reserve_bump;
            // Undercollateralized: 6% collateral against a 5% maintenance +
            // 5% buffer requirement, with room to top up but not to close.
            gs.collateral = 3_000_000;
            gs.policy.caps.partial_close_usd_per_action = 0;
            let guard = guard_account_for(&gs);

            let result = run_tick(&guard, 20, 50_000_000, &tick_data(1, 0));
            assert!(result.is_ok(), "reserve_bump={reserve_bump}: {result:?}");

            let after = GuardState::from_bytes(guard.data()).unwrap();
            assert_eq!(
                after.pending,
                Some(Action::EscalateManualReview),
                "reserve_bump={reserve_bump}"
            );
            assert_eq!(
                after.collateral, 3_000_000,
                "collateral must not be credited"
            );
            assert_eq!(after.nonce, 0, "an escalation commits no nonce");
            assert_eq!(after.daily_spent_usd, 0);
            assert!(
                after.pending_ix.is_none(),
                "no signable ix on a Drift guard"
            );
        }
    }

    // ------------------------------------------------------------------
    //  §8.7 Jupiter defensive close — the co-signed answer to a breach
    // ------------------------------------------------------------------

    /// A breaching Jupiter guard whose only affordable action is a partial close.
    ///
    /// The numbers matter, so they are spelled out: notional is
    /// `100 * $50 = $5,000`, maintenance at 500 bps is `$250`, and the fixture's
    /// `$100` of collateral sits under it — a breach. Capping top-ups at zero
    /// pushes §8.2's precedence past step 2 to the close solver, and `$100` is
    /// deliberately well clear of the `$50` fee on a full close, so the solver
    /// finds a real fraction instead of `CannotReachSafeBuffer` (which would
    /// escalate and quietly build nothing to sign).
    fn jupiter_breach_guard() -> GuardState {
        let mut gs = drift_guard_state();
        gs.venue = VENUE_JUPITER;
        gs.authority_req = AuthorityRequirement::CoSigned;
        gs.policy.authority = AuthorityRequirement::CoSigned;
        gs.policy.caps.top_up_usd_per_action = 0; // no reserve to draw on
        gs
    }

    /// The gap this closes: before it, a co-signed guard could only ever hand its
    /// owner a take-profit — it was silent during the breach it exists to answer.
    #[test]
    fn jupiter_breach_builds_a_signable_defensive_close() {
        let guard = guard_account_for(&jupiter_breach_guard());
        let ts = 1_700_000_000;
        assert!(run_tick(&guard, ts, 50_000_000, &tick_data(1, 0)).is_ok());

        let after = GuardState::from_bytes(guard.data()).unwrap();
        let px = after
            .pending_ix
            .expect("a breach must produce something to sign");
        assert_eq!(px.kind, PENDING_IX_JUPITER_DEFENSIVE_CLOSE);
        assert_eq!(px.expected_nonce, 1);
        // §8.4 — the nonce is not committed until the owner signs.
        assert_eq!(after.nonce, 0);
        assert!(matches!(after.pending, Some(Action::PartialClose { .. })));

        // The bytes are the ones `jupiter::build_defensive_close` produces for
        // this position, not merely "some 50 bytes".
        let Some(Action::PartialClose { fraction_bps }) = after.pending else {
            unreachable!()
        };
        let notional = after.size.unsigned_abs() * after.current_price / SCALE;
        let expected = build_defensive_close(
            50_000_000,
            notional * fraction_bps / BPS_DENOM,
            false,
            ts,
            ts,
            DEFENSIVE_CLOSE_TTL_SECS,
        )
        .unwrap();
        assert_eq!(px.data, expected);
    }

    /// A long is protected by a stop *below* market, a short by one *above*.
    /// Inverting this builds an order that fires the instant it lands, so the
    /// flag is derived from the position's sign rather than taken on trust.
    #[test]
    fn defensive_close_direction_follows_the_position_sign() {
        // Long.
        let long = guard_account_for(&jupiter_breach_guard());
        assert!(run_tick(&long, 1_700_000_000, 50_000_000, &tick_data(1, 0)).is_ok());
        let long_px = GuardState::from_bytes(long.data())
            .unwrap()
            .pending_ix
            .unwrap();
        assert_eq!(long_px.data[8 + 24], 0, "a long's stop sits below market");

        // Short, mirrored: entry above the mark so it is equally underwater.
        let mut gs = jupiter_breach_guard();
        gs.size = -100_000_000;
        gs.entry = 50_000_000;
        let short = guard_account_for(&gs);
        assert!(run_tick(&short, 1_700_000_000, 50_000_000, &tick_data(1, 0)).is_ok());
        let short_px = GuardState::from_bytes(short.data())
            .unwrap()
            .pending_ix
            .unwrap();
        assert_eq!(short_px.data[8 + 24], 1, "a short's stop sits above market");
    }

    /// The owner signs and lands the instruction at the venue; only then does the
    /// guard's nonce advance and the notional charge against the daily budget.
    #[test]
    fn confirming_a_defensive_close_commits_the_nonce_and_charges_the_budget() {
        let guard = guard_account_for(&jupiter_breach_guard());
        assert!(run_tick(&guard, 1_700_000_000, 50_000_000, &tick_data(1, 0)).is_ok());

        let owner = signer_account(Address::from(VENUE_OWNER));
        let route_config = route_config_account();
        let accounts = [guard.view, owner.view, route_config.view];
        assert!(process_instruction(&PROGRAM_ID, &accounts, &[9u8]).is_ok());

        let after = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(after.nonce, 1, "the owner's signature commits the nonce");
        assert!(after.pending_ix.is_none());
        assert!(after.pending.is_none());
        assert!(
            after.daily_spent_usd > 0,
            "a confirmed close must spend against the daily cap, or a co-signed \
             venue can be walked past its budget one signature at a time"
        );
    }

    /// The budget is what stops a co-signed venue from being walked past its
    /// daily allowance one owner signature at a time. A confirmed close spends,
    /// and once spent the next breach gets an escalation rather than a second
    /// instruction to sign.
    ///
    /// Which gate binds is worth stating plainly: §8.2's `select_action` refuses
    /// first, so the guard builds nothing rather than building a stop the owner
    /// would be rejected for signing. The `within_daily` check inside
    /// `confirm_pending` is the backstop behind it, for a `pending_ix` whose
    /// position grew via `UpdatePosition` after the build.
    #[test]
    fn a_confirmed_close_spends_the_budget_and_the_next_one_is_refused() {
        let mut gs = jupiter_breach_guard();
        // One close fits, two do not — the solver needs to shed more than half
        // the position here, so `2 * closed_usd` exceeds the notional itself.
        gs.policy.caps.daily_total_usd = 5_000_000_000;
        let guard = guard_account_for(&gs);

        assert!(run_tick(&guard, 1_700_000_000, 50_000_000, &tick_data(1, 0)).is_ok());
        let built = GuardState::from_bytes(guard.data()).unwrap();
        let Some(Action::PartialClose { fraction_bps }) = built.pending else {
            panic!("expected a partial close, got {:?}", built.pending);
        };
        assert!(built.pending_ix.is_some());

        let owner = signer_account(Address::from(VENUE_OWNER));
        let route_config = route_config_account();
        let accounts = [guard.view, owner.view, route_config.view];
        assert!(process_instruction(&PROGRAM_ID, &accounts, &[9u8]).is_ok());

        let spent = GuardState::from_bytes(guard.data())
            .unwrap()
            .daily_spent_usd;
        let closed_usd = 5_000_000_000u128 * fraction_bps / BPS_DENOM;
        assert_eq!(spent, closed_usd);
        // Guards against a vacuous pass: if one close no longer exhausted most
        // of the budget, the second tick below would be refused for no reason.
        assert!(
            spent.saturating_mul(2) > gs.policy.caps.daily_total_usd,
            "fixture must leave room for one close but not two"
        );

        // Same breach, same guard, next tick — nothing left to spend. Within
        // `MAX_TICK_AGE_SECS` of the first, or the tick is rejected as stale
        // before selection is ever reached.
        assert!(run_tick(&guard, 1_700_000_005, 50_000_000, &tick_data(2, 0)).is_ok());
        let after = GuardState::from_bytes(guard.data()).unwrap();
        assert_eq!(after.pending, Some(Action::EscalateManualReview));
        assert!(
            after.pending_ix.is_none(),
            "an over-budget breach must not hand the owner a stop to sign"
        );
        assert_eq!(after.nonce, 1, "and it commits no nonce");
        assert_eq!(after.daily_spent_usd, spent, "nor spends more");
    }
}
