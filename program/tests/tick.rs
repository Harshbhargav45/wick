//! End-to-end litesvm test of the §7.2 critical path's autonomous branch.
//!
//! Proves the money claim for the demo: when the guard PDA is the Drift
//! sub-account's delegate, `OnPriceTick` → breach → `select_action` →
//! autonomous dispatch CPIs into Drift's `place_perp_order` **signed by the
//! guard PDA's own seeds** (the ER-delegation authority model, §8.4/§8.6). A
//! mock Drift program models Drift's one invariant that matters here: the
//! order `authority` (index 2) must be a signer AND must equal the `delegate`
//! stored in the user account (index 1), and the order must be reduce-only. It
//! stamps the user account so the test can assert the CPI landed.
//!
//! Requires both `.so`s built first:
//!   cargo build-sbf                          (in program/)
//!   cargo build-sbf                          (in program/mocks/drift/)

use litesvm::LiteSVM;
use solana_account::Account;
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_native_token::LAMPORTS_PER_SOL;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::path::PathBuf;

/// Address where the guard `.so` is deployed for the test.
const PROGRAM_ID: &str = "US517G5965aydkZ46HS38QLi7UQiSojurfbQfKCELFx";
/// Real Velocity (Drift successor) program id — the mock is deployed at this
/// address, matching the address the guard's drift adapter CPIs to.
const DRIFT_PROGRAM_ID: &str = "vELoC1audYbSYVRXn1vPaV8Axoa9oU6BYmNGZZBDZ1P";
const CLOCK_SYSVAR: &str = "SysvarC1ock11111111111111111111111111111111";
const RENT_SYSVAR: &str = "SysvarRent111111111111111111111111111111111";
const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";

const GUARD_SEED: &[u8] = b"guard";
const ROUTE_CONFIG_SEED: &[u8] = b"route_config";

/// Marker the mock Drift program writes into the user account on a reduce.
const REDUCED_MARKER: [u8; 6] = *b"REDUCE";

/// Byte offset of `User.delegate` in Drift's zero-copy `User` layout:
/// 8-byte Anchor discriminator + `authority: Pubkey` (32) => 40.
const DELEGATE_OFFSET: usize = 40;

/// litesvm boots the clock at `MAINNET_DEFAULT_SLOT` (~435M), but the guard
/// initializes `last_check_slot` to 0. A tick arriving at that real slot is
/// judged stale (> `MAX_TICK_AGE_SLOTS`, 25) and never dispatches. Warp the
/// clock to a slot inside the freshness window before the tick so the test
/// exercises the real §7.2 dispatch path instead of the degraded-mode gate.
const TICK_SLOT: u64 = 20;

fn read_so(rel: &str) -> Vec<u8> {
    let mut so_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    so_path.push(rel);
    std::fs::read(&so_path).unwrap_or_else(|e| {
        panic!("{so_path:?} not found ({e}). Run `cargo build-sbf` in program/ and program/mocks/drift/ first.")
    })
}

/// InitGuard payload: discriminator 0, bump, then the 150-byte policy blob.
/// Autonomous regime, venue = Drift, tiny top-up cap (forces PartialClose),
/// maintenance at 50% (5000 bp) with 500 bp buffer — the solver test fixture.
fn init_data(bump: u8) -> Vec<u8> {
    let mut data = vec![0u8, bump];
    let mut blob = vec![0u8; 150];
    blob[0] = 3; // venue = Drift
    blob[1..33].copy_from_slice(&[9u8; 32]); // co_authority
    blob[33] = 0; // authority_req = Autonomous
    blob[34..50].copy_from_slice(&5000u128.to_le_bytes()); // maintenance_bps
    blob[50..66].copy_from_slice(&500u128.to_le_bytes()); // trigger_buffer_bps
    blob[66..82].copy_from_slice(&10u128.to_le_bytes()); // fee_bps
    blob[82..98].copy_from_slice(&1u128.to_le_bytes()); // cap_top_up (forces partial close)
    blob[98..114].copy_from_slice(&u128::MAX.to_le_bytes()); // cap_partial_close
    blob[114..130].copy_from_slice(&u128::MAX.to_le_bytes()); // cap_daily
    blob[130..146].copy_from_slice(&u128::MAX.to_le_bytes()); // no take_profit
    blob[146..148].copy_from_slice(&0u16.to_le_bytes()); // drift_market_index (perp market 0)
    blob[148..150].copy_from_slice(&0u16.to_le_bytes()); // drift_subaccount_id
    data.extend_from_slice(&blob);
    data
}

/// Build the mock Drift `user` account: owned by the mock program, with its
/// `delegate` field (offset 32) set to the guard PDA. This is the account the
/// guard's `place_perp_order` CPI addresses as `user`.
fn drift_user_account(drift_pubkey: Pubkey, delegate: Address) -> Account {
    let mut data = vec![0u8; 96];
    data[DELEGATE_OFFSET..DELEGATE_OFFSET + 32].copy_from_slice(delegate.as_ref());
    Account {
        lamports: 1_000_000,
        data,
        owner: drift_pubkey,
        executable: false,
        rent_epoch: 0,
    }
}

/// Pyth pull-oracle constants mirrored from `crate::pyth` (the program is
/// deployed at a fixed address; the SOL/USD feed id and PriceUpdateV2
/// discriminator are read-only constants).
const WICK_PYTH_PROGRAM: Pubkey = Pubkey::new_from_array([
    12, 183, 250, 187, 82, 247, 166, 72, 187, 91, 49, 125, 154, 1, 139, 144, 87, 203, 2, 71, 116,
    250, 254, 1, 230, 196, 223, 152, 204, 56, 88, 129,
]);
const WICK_PYTH_DISCRIMINATOR: [u8; 8] = [34, 241, 35, 99, 157, 126, 244, 205];
const SOL_USD_FEED_ID: [u8; 32] = [
    239, 13, 139, 111, 218, 44, 235, 164, 29, 161, 93, 64, 149, 209, 218, 57, 42, 13, 47, 142, 208,
    198, 199, 188, 15, 76, 250, 200, 194, 128, 181, 109,
];

/// A Pyth `PriceUpdateV2` account (SOL/USD, Full-verified) carrying `price6`
/// in Wick's 6-decimal scale, owned by the Pyth receiver program. With
/// expo=-8, raw * 10^(expo+6) == price6 ⇒ raw = price6*100. publish_time 0 is
/// fresh against the guard's `now` gate (max age 60s params; the clock's
/// unix_timestamp in the litesvm fixture is small). Confidence 0 ≤ 150bps.
fn pyth_account(price6: u128) -> Account {
    let mut data = vec![0u8; 200];
    data[..8].copy_from_slice(&WICK_PYTH_DISCRIMINATOR);
    data[40] = 1; // Full verification
    data[41..73].copy_from_slice(&SOL_USD_FEED_ID);
    data[73..81].copy_from_slice(&((price6 * 100) as i64).to_le_bytes()); // raw
    data[81..89].copy_from_slice(&0u64.to_le_bytes()); // conf
    data[89..93].copy_from_slice(&(-8i32).to_le_bytes()); // expo
    data[93..101].copy_from_slice(&0i64.to_le_bytes()); // publish_time
    Account {
        lamports: 1_000_000,
        data,
        owner: WICK_PYTH_PROGRAM,
        executable: false,
        rent_epoch: 0,
    }
}

/// Derive the singleton RouteConfig PDA: the program seeds it
/// `[b"route_config", bump]`, so the canonical bump append matches.
fn route_config_pda(program_id: Address) -> (Address, u8) {
    Address::find_program_address(&[ROUTE_CONFIG_SEED], &program_id)
}

/// Initialize the singleton RouteConfig via `InitRouteConfig` (disc 10).
/// Layout: [0] config PDA (writable), [1] authority (signer), [2] payer
/// (signer, writable), [3] rent sysvar. Data: [disc, bump].
fn init_route_config(
    svm: &mut LiteSVM,
    program_id: Address,
    rent: Address,
    system: Address,
    owner_kp: &Keypair,
    owner: Address,
) -> (Address, u8) {
    let (config, bump) = route_config_pda(program_id);
    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(config, false),
            AccountMeta::new_readonly(owner, true),
            AccountMeta::new(owner, true),
            AccountMeta::new_readonly(rent, false),
            AccountMeta::new_readonly(system, false),
        ],
        data: vec![10u8, bump],
    };
    let msg = Message::new_with_blockhash(&[ix], Some(&owner), &svm.latest_blockhash());
    let tx = Transaction::new(&[owner_kp], msg, svm.latest_blockhash());
    svm.send_transaction(tx).expect("InitRouteConfig failed");
    (config, bump)
}

#[test]
fn autonomous_tick_cpis_place_perp_order_signed_by_guard_pda() {
    let program_id = Address::from_str_const(PROGRAM_ID);
    let drift_id = Address::from_str_const(DRIFT_PROGRAM_ID);
    let clock = Address::from_str_const(CLOCK_SYSVAR);
    let rent = Address::from_str_const(RENT_SYSVAR);
    let system = Address::from_str_const(SYSTEM_PROGRAM);

    let mut svm = LiteSVM::new().with_sigverify(false);
    svm.add_program(program_id, &read_so("target/deploy/wick_guard.so"))
        .unwrap();
    svm.add_program(
        drift_id,
        &read_so("mocks/drift/target/deploy/mock_drift.so"),
    )
    .unwrap();

    let owner_kp = Keypair::new();
    let owner = owner_kp.pubkey();
    svm.airdrop(&owner, 100 * LAMPORTS_PER_SOL).unwrap();

    let seeds: &[&[u8]] = &[GUARD_SEED, owner.as_ref()];
    let (guard_pda, bump) = Address::find_program_address(seeds, &program_id);

    // --- InitGuard (venue = Drift, Autonomous) ---
    let init_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(guard_pda, false),
            AccountMeta::new_readonly(owner, true),
            AccountMeta::new(owner, true),
            AccountMeta::new_readonly(rent, false),
            AccountMeta::new_readonly(system, false),
        ],
        data: init_data(bump),
    };
    let msg = Message::new_with_blockhash(&[init_ix], Some(&owner), &svm.latest_blockhash());
    let tx = Transaction::new(&[&owner_kp], msg, svm.latest_blockhash());
    svm.send_transaction(tx).expect("InitGuard failed");

    // --- InitRouteConfig (kill-switch singleton, required by every guard ix) ---
    let (route_config, _) = init_route_config(&mut svm, program_id, rent, system, &owner_kp, owner);

    // --- UpdatePosition: collateral $45, size 100, entry $50 (underwater) ---
    let mut upd = vec![8u8];
    upd.extend_from_slice(&45_000_000u128.to_le_bytes());
    upd.extend_from_slice(&100_000_000i128.to_le_bytes());
    upd.extend_from_slice(&50_000_000u128.to_le_bytes());
    let upd_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(guard_pda, false),
            AccountMeta::new_readonly(owner, true),
            AccountMeta::new_readonly(route_config, false),
        ],
        data: upd,
    };
    let msg = Message::new_with_blockhash(&[upd_ix], Some(&owner), &svm.latest_blockhash());
    let tx = Transaction::new(&[&owner_kp], msg, svm.latest_blockhash());
    svm.send_transaction(tx).expect("UpdatePosition failed");

    // --- Drift accounts. The guard passes the tail (state/user/authority +
    // remaining) straight through the place_perp_order CPI. The mock only
    // touches `user` (delegate read + marker write) and `authority` (signer +
    // delegate match). `user` must be owned by the mock program for the write
    // to be legal, and its `delegate` field must be the guard PDA. ---
    let drift_pubkey = Pubkey::from(drift_id.to_bytes());
    let state_acc = Account {
        lamports: 1_000_000,
        data: vec![0u8; 64],
        owner: drift_pubkey,
        executable: false,
        rent_epoch: 0,
    };
    let user_acc = drift_user_account(drift_pubkey, guard_pda);

    let state = Address::from([30u8; 32]);
    let user = Address::from([31u8; 32]);
    let remaining_a = Address::from([40u8; 32]); // perp market map
    let remaining_b = Address::from([41u8; 32]); // oracle map

    svm.set_account(state, state_acc).unwrap();
    svm.set_account(user, user_acc).unwrap();
    for a in [remaining_a, remaining_b] {
        svm.set_account(
            a,
            Account {
                lamports: 1_000_000,
                data: vec![],
                owner: drift_pubkey,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
    }

    // --- Pyth account (authoritative price source at index [3]) ---
    let pyth = Address::from([50u8; 32]);
    svm.set_account(pyth, pyth_account(50_000_000)).unwrap();

    // --- OnPriceTick: price reads from the Pyth oracle ($50 == entry), nonce
    // 1. Underwater position → TopUp capped out → PartialClose → autonomous
    // Drift reduce. Price is NOT in the payload (security: no caller price). ---
    svm.warp_to_slot(TICK_SLOT); // fresh: TICK_SLOT - 0 <= MAX_TICK_AGE_SLOTS
    let mut tick = vec![7u8];
    tick.extend_from_slice(&1u64.to_le_bytes());
    tick.push(bump);
    let tick_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(guard_pda, false),             // [0] guard
            AccountMeta::new_readonly(clock, false),        // [1] clock
            AccountMeta::new_readonly(route_config, false), // [2] route_config
            AccountMeta::new_readonly(pyth, false),         // [3] Pyth PriceUpdateV2
            AccountMeta::new_readonly(state, false),        // [4] Drift state (readonly)
            AccountMeta::new(user, false), // [5] Drift user (delegate read + marker)
            AccountMeta::new(guard_pda, false), // [6] authority (guard PDA — delegate)
            AccountMeta::new_readonly(remaining_a, false), // [7] remaining (perp market map)
            AccountMeta::new_readonly(remaining_b, false), // [8] remaining (oracle map)
            // The CPI target must be present as an account in the transaction
            // message, otherwise the runtime rejects the CPI with
            // `InstructionError::MissingAccount` ("Unknown program").
            AccountMeta::new_readonly(drift_id, false),
        ],
        data: tick,
    };
    let msg = Message::new_with_blockhash(&[tick_ix], Some(&owner), &svm.latest_blockhash());
    let tx = Transaction::new(&[&owner_kp], msg, svm.latest_blockhash());
    let res = svm.send_transaction(tx);
    assert!(res.is_ok(), "OnPriceTick failed: {res:?}");

    // --- Assertions ---
    // The mock Drift program stamped the user account → the guard-PDA-signed
    // reduce-only place_perp_order CPI really landed.
    let user_after = svm.get_account(&user).expect("user missing");
    assert_eq!(
        &user_after.data[..6],
        &REDUCED_MARKER,
        "place_perp_order CPI did not reach the venue"
    );

    // §8.4: autonomous execution commits the nonce.
    let guard_after = svm.get_account(&guard_pda).expect("guard missing");
    let nonce = u64::from_le_bytes(guard_after.data[243..251].try_into().unwrap());
    assert_eq!(nonce, 1, "nonce must commit on autonomous execution");

    // The breach price is reflected in the snapshot.
    let price = u128::from_le_bytes(guard_after.data[227..243].try_into().unwrap());
    assert_eq!(price, 50_000_000);
}

#[test]
fn cosigned_tick_never_reaches_venue() {
    // A CoSigned guard must NOT CPI into the venue at all — no owner signature
    // is present, so the guard can only build + hold the action (§8.4).
    let program_id = Address::from_str_const(PROGRAM_ID);
    let drift_id = Address::from_str_const(DRIFT_PROGRAM_ID);
    let clock = Address::from_str_const(CLOCK_SYSVAR);
    let rent = Address::from_str_const(RENT_SYSVAR);
    let system = Address::from_str_const(SYSTEM_PROGRAM);

    let mut svm = LiteSVM::new().with_sigverify(false);
    svm.add_program(program_id, &read_so("target/deploy/wick_guard.so"))
        .unwrap();
    svm.add_program(
        drift_id,
        &read_so("mocks/drift/target/deploy/mock_drift.so"),
    )
    .unwrap();

    let owner_kp = Keypair::new();
    let owner = owner_kp.pubkey();
    svm.airdrop(&owner, 100 * LAMPORTS_PER_SOL).unwrap();

    let seeds: &[&[u8]] = &[GUARD_SEED, owner.as_ref()];
    let (guard_pda, bump) = Address::find_program_address(seeds, &program_id);

    // CoSigned regime: same numbers but blob[33] = 1.
    let mut data = init_data(bump);
    data[2 + 33] = 1; // payload blob starts at index 2
    let init_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(guard_pda, false),
            AccountMeta::new_readonly(owner, true),
            AccountMeta::new(owner, true),
            AccountMeta::new_readonly(rent, false),
            AccountMeta::new_readonly(system, false),
        ],
        data,
    };
    let msg = Message::new_with_blockhash(&[init_ix], Some(&owner), &svm.latest_blockhash());
    let tx = Transaction::new(&[&owner_kp], msg, svm.latest_blockhash());
    svm.send_transaction(tx).expect("InitGuard failed");

    // --- InitRouteConfig (kill-switch singleton, required by every guard ix) ---
    let (route_config, _) = init_route_config(&mut svm, program_id, rent, system, &owner_kp, owner);

    let mut upd = vec![8u8];
    upd.extend_from_slice(&45_000_000u128.to_le_bytes());
    upd.extend_from_slice(&100_000_000i128.to_le_bytes());
    upd.extend_from_slice(&50_000_000u128.to_le_bytes());
    let upd_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(guard_pda, false),
            AccountMeta::new_readonly(owner, true),
            AccountMeta::new_readonly(route_config, false),
        ],
        data: upd,
    };
    let msg = Message::new_with_blockhash(&[upd_ix], Some(&owner), &svm.latest_blockhash());
    let tx = Transaction::new(&[&owner_kp], msg, svm.latest_blockhash());
    svm.send_transaction(tx).expect("UpdatePosition failed");

    // Tick with the SAME account list as the autonomous test — if the guard
    // wrongly CPI'd, the mock would stamp the user account.
    svm.warp_to_slot(TICK_SLOT); // fresh: TICK_SLOT - 0 <= MAX_TICK_AGE_SLOTS
    let drift_pubkey = Pubkey::from(drift_id.to_bytes());
    let user = Address::from([31u8; 32]);
    svm.set_account(user, drift_user_account(drift_pubkey, guard_pda))
        .unwrap();
    // Pyth oracle at index [3] (authoritative price source).
    let pyth = Address::from([50u8; 32]);
    svm.set_account(pyth, pyth_account(50_000_000)).unwrap();

    let mut tick = vec![7u8];
    tick.extend_from_slice(&1u64.to_le_bytes());
    tick.push(bump);
    let tick_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(guard_pda, false),
            AccountMeta::new_readonly(clock, false),
            AccountMeta::new_readonly(route_config, false), // [2] route_config
            AccountMeta::new_readonly(pyth, false),         // [3] Pyth PriceUpdateV2
            AccountMeta::new_readonly(Address::from([30u8; 32]), false), // [4] state
            AccountMeta::new(user, false),
            AccountMeta::new(guard_pda, false), // authority (guard PDA)
            AccountMeta::new_readonly(Address::from([40u8; 32]), false), // remaining
            AccountMeta::new_readonly(Address::from([41u8; 32]), false), // remaining
            AccountMeta::new_readonly(drift_id, false),
        ],
        data: tick,
    };
    let msg = Message::new_with_blockhash(&[tick_ix], Some(&owner), &svm.latest_blockhash());
    let tx = Transaction::new(&[&owner_kp], msg, svm.latest_blockhash());
    let res = svm.send_transaction(tx);
    assert!(res.is_ok(), "OnPriceTick failed: {res:?}");

    // Venue never reached: user untouched, nonce not committed (§8.4).
    let user_after = svm.get_account(&user).expect("user missing");
    assert_ne!(&user_after.data[..6], &REDUCED_MARKER);
    let guard_after = svm.get_account(&guard_pda).expect("guard missing");
    let nonce = u64::from_le_bytes(guard_after.data[243..251].try_into().unwrap());
    assert_eq!(nonce, 0, "CoSigned must not commit the nonce");

    // Pending action stored for the frontend to co-sign (tag at offset 259).
    assert_ne!(
        guard_after.data[259], 0,
        "pending action must be held for co-sign"
    );
}
