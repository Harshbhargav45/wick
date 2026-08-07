//! End-to-end litesvm test of the §7.2 critical path's autonomous branch.
//!
//! Proves the money claim for the demo: when the guard PDA is the Flash
//! position owner, `OnPriceTick` → breach → `select_action` → autonomous
//! dispatch CPI into Flash's `close_position` **signed by the guard PDA's own
//! seeds** (the ER-delegation authority model, §8.4/§8.6). A mock Flash program
//! models Flash's one invariant that matters here: `owner` (index 0) must be a
//! signer. It stamps the position account so the test can assert the CPI
//! landed.
//!
//! Requires both `.so`s built first:
//!   cargo build-sbf                          (in program/)
//!   cargo build-sbf                          (in program/mocks/flash/)

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
/// Real Flash Perpetuals program id — the mock is deployed at this address.
const FLASH_PROGRAM_ID: &str = "FLASH6Lo6h3iasJKWDs2F8TkW2UKf3s15C8PMGuVfgBn";
const CLOCK_SYSVAR: &str = "SysvarC1ock11111111111111111111111111111111";
const RENT_SYSVAR: &str = "SysvarRent111111111111111111111111111111111";
const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";

const GUARD_SEED: &[u8] = b"guard";

/// Marker the mock Flash program writes into the position on a close.
const CLOSED_MARKER: [u8; 4] = *b"WICK";

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
        panic!("{so_path:?} not found ({e}). Run `cargo build-sbf` in program/ and program/mocks/flash/ first.")
    })
}

/// InitGuard payload: discriminator 0, bump, then the 146-byte policy blob.
/// Autonomous regime, venue = Flash, tiny top-up cap (forces PartialClose),
/// maintenance at 50% (5000 bp) with 500 bp buffer — the solver test fixture.
fn init_data(bump: u8) -> Vec<u8> {
    let mut data = vec![0u8, bump];
    let mut blob = vec![0u8; 146];
    blob[0] = 1; // venue = Flash
    blob[1..33].copy_from_slice(&[9u8; 32]); // co_authority
    blob[33] = 0; // authority_req = Autonomous
    blob[34..50].copy_from_slice(&5000u128.to_le_bytes()); // maintenance_bps
    blob[50..66].copy_from_slice(&500u128.to_le_bytes()); // trigger_buffer_bps
    blob[66..82].copy_from_slice(&10u128.to_le_bytes()); // fee_bps
    blob[82..98].copy_from_slice(&1u128.to_le_bytes()); // cap_top_up (forces partial close)
    blob[98..114].copy_from_slice(&u128::MAX.to_le_bytes()); // cap_partial_close
    blob[114..130].copy_from_slice(&u128::MAX.to_le_bytes()); // cap_daily
    blob[130..146].copy_from_slice(&u128::MAX.to_le_bytes()); // no take_profit
    data.extend_from_slice(&blob);
    data
}

#[test]
fn autonomous_tick_cpis_close_position_signed_by_guard_pda() {
    let program_id = Address::from_str_const(PROGRAM_ID);
    let flash_id = Address::from_str_const(FLASH_PROGRAM_ID);
    let clock = Address::from_str_const(CLOCK_SYSVAR);
    let rent = Address::from_str_const(RENT_SYSVAR);
    let system = Address::from_str_const(SYSTEM_PROGRAM);

    let mut svm = LiteSVM::new().with_sigverify(false);
    svm.add_program(program_id, &read_so("target/deploy/wick_guard.so"))
        .unwrap();
    svm.add_program(
        flash_id,
        &read_so("mocks/flash/target/deploy/mock_flash.so"),
    )
    .unwrap();

    let owner_kp = Keypair::new();
    let owner = owner_kp.pubkey();
    svm.airdrop(&owner, 100 * LAMPORTS_PER_SOL).unwrap();

    let seeds: &[&[u8]] = &[GUARD_SEED, owner.as_ref()];
    let (guard_pda, bump) = Address::find_program_address(seeds, &program_id);

    // --- InitGuard (venue = Flash, Autonomous) ---
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

    // --- Flash accounts. The guard program passes these straight through the
    // close_position CPI; the mock only touches `owner` (signer) and
    // `position` (index 5). They must exist and be owned by the flash program
    // for the write to be legal. ---
    let flash_pubkey = Pubkey::from(flash_id.to_bytes());
    let dummy_meta = Account {
        lamports: 1_000_000,
        data: vec![],
        owner: flash_pubkey,
        executable: false,
        rent_epoch: 0,
    };
    let position_acc = Account {
        lamports: 1_000_000,
        data: vec![0u8; 8],
        owner: flash_pubkey,
        executable: false,
        rent_epoch: 0,
    };

    let position = Address::from([42u8; 32]);
    let receiving = Address::from([11u8; 32]);
    let transfer_authority = Address::from([12u8; 32]);
    let perpetuals = Address::from([13u8; 32]);
    let pool = Address::from([14u8; 32]);
    let custody = Address::from([15u8; 32]);
    let custody_oracle = Address::from([16u8; 32]);
    let collateral_custody = Address::from([17u8; 32]);
    let collateral_oracle = Address::from([18u8; 32]);
    let collateral_token = Address::from([19u8; 32]);
    let token_program = Address::from([20u8; 32]);

    svm.set_account(position, position_acc).unwrap();
    for a in [
        receiving,
        transfer_authority,
        perpetuals,
        pool,
        custody,
        custody_oracle,
        collateral_custody,
        collateral_oracle,
        collateral_token,
        token_program,
    ] {
        svm.set_account(a, dummy_meta.clone()).unwrap();
    }

    // --- OnPriceTick: price $50 (entry), nonce 1. Underwater position →
    // TopUp capped out → PartialClose → autonomous Flash close. ---
    svm.warp_to_slot(TICK_SLOT); // fresh: TICK_SLOT - 0 <= MAX_TICK_AGE_SLOTS
    let mut tick = vec![7u8];
    tick.extend_from_slice(&50_000_000u128.to_le_bytes());
    tick.extend_from_slice(&1u64.to_le_bytes());
    tick.push(bump);
    let tick_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(guard_pda, false),      // [0] guard
            AccountMeta::new_readonly(clock, false), // [1] clock
            AccountMeta::new(guard_pda, false),      // [2] flash owner (guard PDA)
            AccountMeta::new(receiving, false),      // [3] receiving_account
            AccountMeta::new_readonly(transfer_authority, false),
            AccountMeta::new_readonly(perpetuals, false),
            AccountMeta::new(pool, false),
            AccountMeta::new(position, false), // [7] position (mock stamps this)
            AccountMeta::new(custody, false),
            AccountMeta::new_readonly(custody_oracle, false),
            AccountMeta::new(collateral_custody, false),
            AccountMeta::new_readonly(collateral_oracle, false),
            AccountMeta::new(collateral_token, false),
            AccountMeta::new_readonly(token_program, false),
            // The CPI target must be present as an account in the transaction
            // message, otherwise the runtime rejects the CPI with
            // `InstructionError::MissingAccount` ("Unknown program").
            AccountMeta::new_readonly(flash_id, false),
        ],
        data: tick,
    };
    let msg = Message::new_with_blockhash(&[tick_ix], Some(&owner), &svm.latest_blockhash());
    let tx = Transaction::new(&[&owner_kp], msg, svm.latest_blockhash());
    let res = svm.send_transaction(tx);
    assert!(res.is_ok(), "OnPriceTick failed: {res:?}");

    // --- Assertions ---
    // The mock Flash program stamped the position → the guard-PDA-signed
    // close_position CPI really landed.
    let position_after = svm.get_account(&position).expect("position missing");
    assert_eq!(
        &position_after.data[..4],
        &CLOSED_MARKER,
        "close_position CPI did not reach the venue"
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
    let flash_id = Address::from_str_const(FLASH_PROGRAM_ID);
    let clock = Address::from_str_const(CLOCK_SYSVAR);
    let rent = Address::from_str_const(RENT_SYSVAR);
    let system = Address::from_str_const(SYSTEM_PROGRAM);

    let mut svm = LiteSVM::new().with_sigverify(false);
    svm.add_program(program_id, &read_so("target/deploy/wick_guard.so"))
        .unwrap();
    svm.add_program(
        flash_id,
        &read_so("mocks/flash/target/deploy/mock_flash.so"),
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
    // wrongly CPI'd, the mock would stamp the position.
    svm.warp_to_slot(TICK_SLOT); // fresh: TICK_SLOT - 0 <= MAX_TICK_AGE_SLOTS
    let position = Address::from([42u8; 32]);
    let flash_pubkey = Pubkey::from(flash_id.to_bytes());
    svm.set_account(
        position,
        Account {
            lamports: 1_000_000,
            data: vec![0u8; 8],
            owner: flash_pubkey,
            executable: false,
            rent_epoch: 0,
        },
    )
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
            AccountMeta::new(guard_pda, false),
            AccountMeta::new(Address::from([11u8; 32]), false),
            AccountMeta::new_readonly(Address::from([12u8; 32]), false),
            AccountMeta::new_readonly(Address::from([13u8; 32]), false),
            AccountMeta::new(Address::from([14u8; 32]), false),
            AccountMeta::new(position, false),
            AccountMeta::new(Address::from([15u8; 32]), false),
            AccountMeta::new_readonly(Address::from([16u8; 32]), false),
            AccountMeta::new(Address::from([17u8; 32]), false),
            AccountMeta::new_readonly(Address::from([18u8; 32]), false),
            AccountMeta::new(Address::from([19u8; 32]), false),
            AccountMeta::new_readonly(Address::from([20u8; 32]), false),
        ],
        data: tick,
    };
    let msg = Message::new_with_blockhash(&[tick_ix], Some(&owner), &svm.latest_blockhash());
    let tx = Transaction::new(&[&owner_kp], msg, svm.latest_blockhash());
    let res = svm.send_transaction(tx);
    assert!(res.is_ok(), "OnPriceTick failed: {res:?}");

    // Venue never reached: position untouched, nonce not committed (§8.4).
    let position_after = svm.get_account(&position).expect("position missing");
    assert_ne!(&position_after.data[..4], &CLOSED_MARKER);
    let guard_after = svm.get_account(&guard_pda).expect("guard missing");
    let nonce = u64::from_le_bytes(guard_after.data[243..251].try_into().unwrap());
    assert_eq!(nonce, 0, "CoSigned must not commit the nonce");

    // Pending action stored for the frontend to co-sign (tag at offset 259).
    assert_ne!(
        guard_after.data[259], 0,
        "pending action must be held for co-sign"
    );
}
