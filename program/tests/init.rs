//! End-to-end integration test against a real SBF VM (LiteSVM), exercising
//! the CPI-backed `InitGuard` handler and the deposit/withdraw flow.
//!
//! Loads `wick_guard.so` produced by `cargo build-sbf`.

use litesvm::LiteSVM;
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_native_token::LAMPORTS_PER_SOL;
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::path::PathBuf;

/// Address where the program `.so` is deployed for the test. Arbitrary — the
/// loader treats this as the program id regardless of build-time settings.
const PROGRAM_ID: &str = "US517G5965aydkZ46HS38QLi7UQiSojurfbQfKCELFx";

/// Solana rent sysvar program address.
const RENT_SYSVAR: &str = "SysvarRent111111111111111111111111111111111";

/// System program address (required by the CPI inside InitGuard).
const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";

const GUARD_SEED: &[u8] = b"guard";

fn read_wick_program() -> Vec<u8> {
    let mut so_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    so_path.push("target/deploy/wick_guard.so");
    std::fs::read(&so_path).unwrap_or_else(|e| {
        panic!("wick_guard.so not found at {so_path:?} ({e}). Run `cargo build-sbf` first.")
    })
}

/// Build the InitGuard payload: discriminator 0, bump, then the 145-byte
/// policy blob (see `processor::parse_policy`).
fn init_data(bump: u8) -> Vec<u8> {
    let mut data = vec![0u8, bump];
    let mut blob = vec![0u8; 145];
    blob[0..32].copy_from_slice(&[9u8; 32]); // co_authority
    blob[32] = 1; // authority_req = CoSigned
    blob[33..49].copy_from_slice(&500u128.to_le_bytes()); // maintenance_bps
    blob[49..65].copy_from_slice(&500u128.to_le_bytes()); // trigger_buffer_bps
    blob[65..81].copy_from_slice(&10u128.to_le_bytes()); // fee_bps
    blob[81..97].copy_from_slice(&1_000_000u128.to_le_bytes()); // cap_top_up
    blob[97..113].copy_from_slice(&1_000_000u128.to_le_bytes()); // cap_partial_close
    blob[113..129].copy_from_slice(&5_000_000u128.to_le_bytes()); // cap_daily
    blob[129..145].copy_from_slice(&u128::MAX.to_le_bytes()); // no take_profit
    data.extend_from_slice(&blob);
    data
}

#[test]
fn init_guard_and_deposit() {
    let program_id = Address::from_str_const(PROGRAM_ID);
    let rent_sysvar = Address::from_str_const(RENT_SYSVAR);
    let system_program = Address::from_str_const(SYSTEM_PROGRAM);

    let mut svm = LiteSVM::new().with_sigverify(false);
    svm.add_program(program_id, &read_wick_program()).unwrap();

    let owner_kp = Keypair::new();
    let owner = owner_kp.pubkey();
    svm.airdrop(&owner, 100 * LAMPORTS_PER_SOL).unwrap();

    // Derive the guard PDA: [b"guard", owner].
    let seeds: &[&[u8]] = &[GUARD_SEED, owner.as_ref()];
    let (guard_pda, bump) = Address::find_program_address(seeds, &program_id);

    // --- InitGuard ---
    let init_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(guard_pda, false),
            AccountMeta::new_readonly(owner, true),
            AccountMeta::new(owner, true), // payer == owner (signer, writable)
            AccountMeta::new_readonly(rent_sysvar, false),
            AccountMeta::new_readonly(system_program, false),
        ],
        data: init_data(bump),
    };
    let msg = Message::new_with_blockhash(&[init_ix], Some(&owner), &svm.latest_blockhash());
    let tx = Transaction::new(&[&owner_kp], msg, svm.latest_blockhash());
    let res = svm.send_transaction(tx);
    assert!(res.is_ok(), "InitGuard failed: {res:?}");

    let guard_account = svm.get_account(&guard_pda).expect("guard account missing");
    assert_eq!(guard_account.data[0], 1, "guard not initialized with version badge");
    assert_eq!(guard_account.owner, program_id, "guard not owned by program");

    // --- Deposit 1_000_000_000 ---
    let mut deposit_data = vec![1u8]; // discriminator
    deposit_data.extend_from_slice(&1_000_000_000u128.to_le_bytes());
    let deposit_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(guard_pda, false),
            AccountMeta::new_readonly(owner, true),
        ],
        data: deposit_data,
    };
    let deposit_msg =
        Message::new_with_blockhash(&[deposit_ix], Some(&owner), &svm.latest_blockhash());
    let deposit_tx = Transaction::new(&[&owner_kp], deposit_msg, svm.latest_blockhash());
    svm.send_transaction(deposit_tx).expect("DepositMargin failed");
}