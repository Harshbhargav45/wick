//! Latency benchmark for the §7.2 autonomous tick path.
//!
//! Measures guard `OnPriceTick` → reduce-only `place_perp_order` CPI dispatch
//! latency against the **real** Velocity program (same fixture/choreography as
//! `real_drift`), over a burst of warm iterations. Writes the raw per-tick
//! samples (µs) to a JSON file the dashboard renders as the honest latency
//! chart.
//!
//! This is `#[ignore]`d from CI: it's a measurement harness, not a pass/fail
//! assertion. Run it explicitly to refresh the dashboard dataset:
//!
//! ```text
//! cargo build-sbf && cargo test --test latency_bench -- --ignored --nocapture
//! ```

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
use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Bench settings — burst size and where the sample JSON lands.
const ITERATIONS: usize = 300;
const OUTPUT_REL: &str = "../frontend/public/latency-samples.json";

const PROGRAM_ID: &str = "US517G5965aydkZ46HS38QLi7UQiSojurfbQfKCELFx";
const DRIFT_PROGRAM_ID: &str = "vELoC1audYbSYVRXn1vPaV8Axoa9oU6BYmNGZZBDZ1P";
const CLOCK_SYSVAR: &str = "SysvarC1ock11111111111111111111111111111111";
const RENT_SYSVAR: &str = "SysvarRent111111111111111111111111111111111";
const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
const GUARD_SEED: &[u8] = b"guard";
const TICK_SLOT: u64 = 438_024_310;

const USER_DISCRIMINATOR: [u8; 8] = [0x9f, 0x75, 0x5f, 0xe3, 0xef, 0x97, 0x3a, 0xec];
const USER_SIZE: usize = 4496;
const DELEGATE_OFFSET: usize = 40;
const SPOT_POSITIONS_OFFSET: usize = 104;
const PERP_POSITIONS_OFFSET: usize = 424;
const NEXT_ORDER_ID_OFFSET: usize = 4456;
const SPOT_SCALED_BALANCE: u64 = 1_000_000_000_000_000;
const PERP_BASE_ASSET_AMOUNT: i64 = 100_000_000;
const PERP_POSITION_MARKET_INDEX_OFFSET: usize = 76;

fn read_fixture(rel: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures");
    p.push(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("fixture {p:?} missing ({e})"))
}

fn decompress_velocity_so() -> Vec<u8> {
    let gz = read_fixture("velocity.so.gz");
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(&gz[..])
        .read_to_end(&mut out)
        .expect("decompressing velocity.so.gz");
    out
}

fn read_so(rel: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{p:?} not found ({e})"))
}

fn init_data(bump: u8) -> Vec<u8> {
    let mut data = vec![0u8, bump];
    let mut blob = vec![0u8; 150];
    blob[0] = 3; // venue = velocity
    blob[1..33].copy_from_slice(&[9u8; 32]);
    blob[33] = 0; // Autonomous
    blob[34..50].copy_from_slice(&5000u128.to_le_bytes());
    blob[50..66].copy_from_slice(&500u128.to_le_bytes());
    blob[66..82].copy_from_slice(&10u128.to_le_bytes());
    blob[82..98].copy_from_slice(&1u128.to_le_bytes());
    blob[98..114].copy_from_slice(&u128::MAX.to_le_bytes());
    blob[114..130].copy_from_slice(&u128::MAX.to_le_bytes());
    blob[130..146].copy_from_slice(&u128::MAX.to_le_bytes());
    blob[146..148].copy_from_slice(&0u16.to_le_bytes());
    blob[148..150].copy_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&blob);
    data
}

fn synthetic_user(delegate: Address) -> Vec<u8> {
    let mut data = vec![0u8; USER_SIZE];
    data[..8].copy_from_slice(&USER_DISCRIMINATOR);
    data[DELEGATE_OFFSET..DELEGATE_OFFSET + 32].copy_from_slice(delegate.as_ref());
    let sp = SPOT_POSITIONS_OFFSET;
    data[sp..sp + 8].copy_from_slice(&SPOT_SCALED_BALANCE.to_le_bytes());
    data[sp + 32..sp + 34].copy_from_slice(&0u16.to_le_bytes());
    data[sp + 34] = 0;
    data[sp + 35] = 0;
    let pp = PERP_POSITIONS_OFFSET;
    data[pp + 8..pp + 16].copy_from_slice(&PERP_BASE_ASSET_AMOUNT.to_le_bytes());
    data[pp + PERP_POSITION_MARKET_INDEX_OFFSET..pp + PERP_POSITION_MARKET_INDEX_OFFSET + 2]
        .copy_from_slice(&0u16.to_le_bytes());
    data[NEXT_ORDER_ID_OFFSET..NEXT_ORDER_ID_OFFSET + 4].copy_from_slice(&1u32.to_le_bytes());
    data
}

#[allow(clippy::too_many_arguments)]
fn set_velocity_accounts(
    svm: &mut LiteSVM,
    drift_pubkey: Pubkey,
    delegate: Address,
) -> (Address, Address, Address, Address, Address, Address) {
    let perp_fx = read_fixture("velocity_perp_market_0.bin");
    let spot_fx = read_fixture("velocity_spot_market_0.bin");
    let sol_oracle_addr = Address::new_from_array(perp_fx[312..344].try_into().unwrap());
    let usdc_oracle_addr = Address::new_from_array(spot_fx[40..72].try_into().unwrap());
    let state_addr = Address::new_from_array([30u8; 32]);
    let perp_addr = Address::new_from_array([31u8; 32]);
    let spot_addr = Address::new_from_array([32u8; 32]);
    let user_addr = Address::new_from_array([33u8; 32]);

    let mk = |data: Vec<u8>| Account {
        lamports: 1_000_000_000,
        data,
        owner: drift_pubkey,
        executable: false,
        rent_epoch: 0,
    };

    svm.set_account(state_addr, mk(read_fixture("velocity_state.bin")))
        .unwrap();
    svm.set_account(perp_addr, mk(read_fixture("velocity_perp_market_0.bin")))
        .unwrap();
    svm.set_account(spot_addr, mk(read_fixture("velocity_spot_market_0.bin")))
        .unwrap();
    svm.set_account(
        sol_oracle_addr,
        mk(read_fixture("velocity_oracle_perp.bin")),
    )
    .unwrap();
    svm.set_account(
        usdc_oracle_addr,
        mk(read_fixture("velocity_oracle_spot.bin")),
    )
    .unwrap();
    svm.set_account(
        user_addr,
        Account {
            lamports: 1_000_000_000,
            data: synthetic_user(delegate),
            owner: drift_pubkey,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();

    (
        state_addr,
        user_addr,
        sol_oracle_addr,
        usdc_oracle_addr,
        spot_addr,
        perp_addr,
    )
}

#[test]
#[ignore = "measurement harness (not CI assertion)"]
fn measure_autonomous_tick_dispatch_latency() {
    let program_id = Address::from_str_const(PROGRAM_ID);
    let drift_id = Address::from_str_const(DRIFT_PROGRAM_ID);
    let clock = Address::from_str_const(CLOCK_SYSVAR);
    let rent = Address::from_str_const(RENT_SYSVAR);
    let system = Address::from_str_const(SYSTEM_PROGRAM);

    let mut svm = LiteSVM::new().with_sigverify(false);
    svm.add_program(program_id, &read_so("target/deploy/wick_guard.so"))
        .unwrap();
    svm.add_program(drift_id, &decompress_velocity_so())
        .unwrap();

    let owner_kp = Keypair::new();
    let owner = owner_kp.pubkey();
    svm.airdrop(&owner, 100 * LAMPORTS_PER_SOL).unwrap();

    let seeds: &[&[u8]] = &[GUARD_SEED, owner.as_ref()];
    let (guard_pda, bump) = Address::find_program_address(seeds, &program_id);

    // --- InitGuard (velocity venue, Autonomous regime) ---
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
    let h = svm.latest_blockhash();
    let tx = Transaction::new(
        &[&owner_kp],
        Message::new_with_blockhash(&[init_ix], Some(&owner), &h),
        h,
    );
    svm.send_transaction(tx).expect("InitGuard failed");

    // --- UpdatePosition: underwater position that a breach tick must reduce ---
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
    let h = svm.latest_blockhash();
    let tx = Transaction::new(
        &[&owner_kp],
        Message::new_with_blockhash(&[upd_ix], Some(&owner), &h),
        h,
    );
    svm.send_transaction(tx).expect("UpdatePosition failed");

    // --- Real Velocity accounts (oracles at their mainnet pubkeys) ---
    let drift_pubkey = Pubkey::from(drift_id.to_bytes());
    let (state, user, oracle_sol, oracle_usdc, spot, perp) =
        set_velocity_accounts(&mut svm, drift_pubkey, guard_pda);

    svm.warp_to_slot(TICK_SLOT);

    let build_tick = || {
        let mut data = vec![7u8];
        data.extend_from_slice(&50_000_000u128.to_le_bytes());
        data.extend_from_slice(&1u64.to_le_bytes());
        data.push(bump);
        Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(guard_pda, false),            // [0] guard state
                AccountMeta::new_readonly(clock, false),       // [1] clock
                AccountMeta::new_readonly(state, false),       // [2] Real State
                AccountMeta::new(user, false),                 // [3] Real User
                AccountMeta::new(guard_pda, false),            // [4] authority = guard PDA
                AccountMeta::new_readonly(oracle_sol, false),  // [5] perp oracle
                AccountMeta::new_readonly(oracle_usdc, false), // [6] spot oracle
                AccountMeta::new_readonly(spot, false),        // [7] spot market 0
                AccountMeta::new_readonly(perp, false),        // [8] perp market 0
                AccountMeta::new_readonly(drift_id, false),    // [9] CPI target
            ],
            data,
        }
    };

    let build_tx = |blockhash| {
        let ix = build_tick();
        Transaction::new(
            &[&owner_kp],
            Message::new_with_blockhash(&[ix], Some(&owner), &blockhash),
            blockhash,
        )
    };

    // Warmup tick (cold-start freshness anchor) — not measured.
    let blockhash = svm.latest_blockhash();
    let (res, _warm_ms) = send_timed(&mut svm, build_tx(blockhash));
    assert!(res.is_ok(), "warmup tick failed: {res:?}");

    // Measured burst: each tick dispatches OnPriceTick → real velocity CPI.
    let mut samples: Vec<u128> = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let blockhash = svm.latest_blockhash();
        let (res, dt) = send_timed(&mut svm, build_tx(blockhash));
        assert!(res.is_ok(), "tick dispatch failed: {res:?}");
        samples.push(dt.as_micros());
    }

    // Aggregate for the chart's summary strip.
    let mut sorted = samples.clone();
    sorted.sort_unstable();
    let p50 = sorted[sorted.len() / 2];
    let p99 = sorted[(sorted.len() * 99) / 100];
    let min = sorted[0];
    let max = *sorted.last().unwrap();

    // Emit a compact dataset for the dashboard (manual JSON, no serde dep).
    let samples_json: String = samples
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let json = format!(
        r#"{{"lanes":{{"l1_slot_ms":400.0,"sub_50ms_target_us":50000.0}},"samples_us":[{samples_json}],"summary_us":{{"min":{min},"p50":{p50},"p99":{p99},"max":{max}}},"note":"litesvm in-process: OnTick -> velocity place_perp_order CPI lands"}}"#,
    );

    let mut out = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    out.push(OUTPUT_REL);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("create output dir");
    }
    std::fs::write(&out, json).expect("write latency samples");

    println!("wrote {} samples -> {}", samples.len(), out.display());
    println!("p50={p50}us p99={p99}us min={min}us max={max}us (L1 slot baseline ~400ms)");

    // The core claim: the median dispatch is far under the 50ms target.
    assert!(
        p50 < 50_000,
        "p50 {p50}us must stay under the 50ms sub-latency target"
    );
}

fn send_timed(
    svm: &mut LiteSVM,
    tx: Transaction,
) -> (
    Result<(), litesvm::types::FailedTransactionMetadata>,
    Duration,
) {
    let start = Instant::now();
    let res = svm.send_transaction(tx);
    let dt = start.elapsed();
    (res.map(|_| ()), dt)
}
