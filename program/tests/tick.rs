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
/// Real Drift Protocol program id — the mock is deployed at this address.
const DRIFT_PROGRAM_ID: &str = "dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH";
const CLOCK_SYSVAR: &str = "SysvarC1ock11111111111111111111111111111111";
const RENT_SYSVAR: &str = "SysvarRent111111111111111111111111111111111";
const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";

const GUARD_SEED: &[u8] = b"guard";

/// Marker the mock Drift program writes into the user account on a reduce.
const REDUCED_MARKER: [u8; 6] = *b"REDUCE";

/// Byte offset of `User.delegate` in Drift's zero-copy `User` layout.
const DELEGATE_OFFSET: usize = 32;

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

    // --- OnPriceTick: price $50 (entry), nonce 1. Underwater position →
    // TopUp capped out → PartialClose → autonomous Drift reduce. ---
    svm.warp_to_slot(TICK_SLOT); // fresh: TICK_SLOT - 0 <= MAX_TICK_AGE_SLOTS
    let mut tick = vec![7u8];
    tick.extend_from_slice(&50_000_000u128.to_le_bytes());
    tick.extend_from_slice(&1u64.to_le_bytes());
    tick.push(bump);
    let tick_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(guard_pda, false),            // [0] guard
            AccountMeta::new_readonly(clock, false),       // [1] clock
            AccountMeta::new_readonly(state, false),       // [2] Drift state (readonly)
            AccountMeta::new(user, false), // [3] Drift user (delegate read + marker)
            AccountMeta::new(guard_pda, false), // [4] authority (guard PDA — delegate)
            AccountMeta::new_readonly(remaining_a, false), // [5] remaining (perp market map)
            AccountMeta::new_readonly(remaining_b, false), // [6] remaining (oracle map)
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

    let mut upd = vec![8u8];
    upd.extend_from_slice(&45_000_000u128.to_le_bytes());
    upd.extend_from_slice(&100_000_000i128.to_le_bytes());
    upd.extend_from_slice(&50_000_000u128.to_le_bytes());
    let upd_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(guard_pda, false),
            AccountMeta::new_readonly(owner, true),
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

    let mut tick = vec![7u8];
    tick.extend_from_slice(&50_000_000u128.to_le_bytes());
    tick.extend_from_slice(&1u64.to_le_bytes());
    tick.push(bump);
    let tick_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(guard_pda, false),
            AccountMeta::new_readonly(clock, false),
            AccountMeta::new_readonly(Address::from([30u8; 32]), false), // state
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
