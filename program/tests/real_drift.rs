//! End-to-end litesvm test against the **real live Velocity program** — the
//! successor to Drift Protocol v2 (the `dRifty...` program was decommissioned
//! after the 2026 exploit; its mainnet binary now only handles withdrawals).
//!
//! Proves the §7.2 critical path's autonomous branch against real on-chain
//! account state, not a mock: the guard PDA signs `place_perp_order` into the
//! actual `vELoC1...` BPF program and the real program accepts the reduce-only
//! order.
//!
//! Approach (§6.5): the test vendors:
//!
//! * the real `velocity.so` (gzip-compressed here; decompressed with `flate2`);
//! * real mainnet account payloads fetched at a fixed slot (see
//!   `program/tests/fixtures/README`): state, perp market 0, spot market 0,
//!   and the two PythLazer oracles referenced by those markets.
//!
//! Clock choreography: the fixtures' oracles were posted at slot ~438,024,296.
//! Velocity gates every order on a fresh oracle (delay = clock.slot -
//! posted_slot must be under the market's stale threshold), so we warp the
//! litesvm clock to a slot just past both oracle posts. The guard's own
//! freshness gate (§8.1) then needs one warmup tick to fold in
//! `last_check_slot` before the dispatch tick is considered fresh.
//!
//! The synthetic `User` mirrors Velocity's zero-copy layout — the only nonzero
//! fields are those that make a real margin check pass and drive the program to
//! store the reduce-only order: `delegate` = guard PDA, a large quote deposit
//! (spot market 0) for collateral, and a perp long on market 0 for the reduce
//! to be sincere.
//!
//! Requires the guard `.so` built first:
//!   cargo build-sbf                          (in program/)

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

/// Address where wick's guard `.so` is deployed for the test (arbitrary).
const PROGRAM_ID: &str = "US517G5965aydkZ46HS38QLi7UQiSojurfbQfKCELFx";
/// Real Velocity program id — the live successor of Drift Protocol v2.
const DRIFT_PROGRAM_ID: &str = "vELoC1audYbSYVRXn1vPaV8Axoa9oU6BYmNGZZBDZ1P";
const CLOCK_SYSVAR: &str = "SysvarC1ock11111111111111111111111111111111";
const RENT_SYSVAR: &str = "SysvarRent111111111111111111111111111111111";
const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";

const GUARD_SEED: &[u8] = b"guard";

/// Clock slot to run the dispatch at: just past both fixtures' oracle
/// `posted_slot` (~438,024,296) so Velocity's money math sees fresh markets.
const TICK_SLOT: u64 = 438_024_310;

/// `User` zero-copy layout constants (velocity IDL, verified against
/// `@velocity-exchange/sdk` IDL account `User`).
const USER_DISCRIMINATOR: [u8; 8] = [0x9f, 0x75, 0x5f, 0xe3, 0xef, 0x97, 0x3a, 0xec];
const USER_SIZE: usize = 4496; // 8-byte disc + 4488 struct
const DELEGATE_OFFSET: usize = 40;
const SPOT_POSITIONS_OFFSET: usize = 104;
const PERP_POSITIONS_OFFSET: usize = 424;
const ORDERS_OFFSET: usize = 1064;
const NEXT_ORDER_ID_OFFSET: usize = 4456;

/// SpotPosition (40 bytes) @ market 0 — a quote (USDT) deposit.
const SPOT_SCALED_BALANCE: u64 = 1_000_000_000_000_000;
/// PerpPosition (80 bytes velocity) on perp market 0: a long of the watched size.
const PERP_BASE_ASSET_AMOUNT: i64 = 100_000_000; // 0.1 base asset
const PERP_POSITION_MARKET_INDEX_OFFSET: usize = 76;

/// Read a test fixture under `program/tests/fixtures`.
fn read_fixture(rel: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures");
    p.push(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("fixture {p:?} missing ({e})"))
}

/// Decompress `velocity.so.gz` into the runnable ELF.
fn decompress_velocity_so() -> Vec<u8> {
    let gz = read_fixture("velocity.so.gz");
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(&gz[..])
        .read_to_end(&mut out)
        .expect("decompressing velocity.so.gz");
    out
}

/// InitGuard payload (mirrors `tick.rs`): discriminator 0, bump, policy blob.
/// Autonomous regime, venue = Drift, tiny top-up cap (forces PartialClose).
fn init_data(bump: u8) -> Vec<u8> {
    let mut data = vec![0u8, bump];
    let mut blob = vec![0u8; 150];
    blob[0] = 3; // venue = Drift (velocity)
    blob[1..33].copy_from_slice(&[9u8; 32]); // co_authority
    blob[33] = 0; // authority_req = Autonomous
    blob[34..50].copy_from_slice(&5000u128.to_le_bytes()); // maintenance_bps
    blob[50..66].copy_from_slice(&500u128.to_le_bytes()); // trigger_buffer_bps
    blob[66..82].copy_from_slice(&10u128.to_le_bytes()); // fee_bps
    blob[82..98].copy_from_slice(&1u128.to_le_bytes()); // cap_top_up (forces partial close)
    blob[98..114].copy_from_slice(&u128::MAX.to_le_bytes()); // cap_partial_close
    blob[114..130].copy_from_slice(&u128::MAX.to_le_bytes()); // cap_daily
    blob[130..146].copy_from_slice(&u128::MAX.to_le_bytes()); // no take_profit
    blob[146..148].copy_from_slice(&0u16.to_le_bytes()); // perp market index 0
    blob[148..150].copy_from_slice(&0u16.to_le_bytes()); // sub_account_id 0
    data.extend_from_slice(&blob);
    data
}

/// Build the synthetic real-layout Velocity `User` (4496 bytes). The
/// `authority` and `delegate` fields are set so the guard PDA can sign as the
/// user's delegate; the big quote deposit satisfies the maintenance margin path
/// a reduce-only order runs; the perp long is what gets reduced.
fn synthetic_user(delegate: Address) -> Vec<u8> {
    let mut data = vec![0u8; USER_SIZE];
    data[..8].copy_from_slice(&USER_DISCRIMINATOR);
    // authority: default (only acts via delegate). delegate = guard PDA.
    data[DELEGATE_OFFSET..DELEGATE_OFFSET + 32].copy_from_slice(delegate.as_ref());
    // spot_positions[0]: deposit market 0. scaled_balance@+0 = big,
    // market_index@+32=0, balance_type@+34=0 (Deposit).
    let sp = SPOT_POSITIONS_OFFSET;
    data[sp..sp + 8].copy_from_slice(&SPOT_SCALED_BALANCE.to_le_bytes());
    data[sp + 32..sp + 34].copy_from_slice(&0u16.to_le_bytes());
    data[sp + 34] = 0; // balance_type = Deposit
    data[sp + 35] = 0; // open_orders
                       // perp_positions[0]: long on perp market 0. base_asset_amount@+8 = size,
                       // market_index@+76 = 0.
    let pp = PERP_POSITIONS_OFFSET;
    data[pp + 8..pp + 16].copy_from_slice(&PERP_BASE_ASSET_AMOUNT.to_le_bytes());
    data[pp + PERP_POSITION_MARKET_INDEX_OFFSET..pp + PERP_POSITION_MARKET_INDEX_OFFSET + 2]
        .copy_from_slice(&0u16.to_le_bytes());
    // next_order_id = 1 (fresh user).
    data[NEXT_ORDER_ID_OFFSET..NEXT_ORDER_ID_OFFSET + 4].copy_from_slice(&1u32.to_le_bytes());
    data[DELEGATE_OFFSET + 32..DELEGATE_OFFSET + 34].copy_from_slice(&[0, 0]); // padding stays zero
    data
}

/// Place the real Velocity account payloads (state, perp market 0, spot market
/// 0, two markets' oracles) plus the synthetic User at seedable addresses.
///
/// Full real-velocity invariants:
/// * the perp oracle account sits at the pubkey stored in the perp fixture
///   (PerpMarket.oracle @ +312) — OracleMap indexes it by pubkey;
/// * the spot oracle account sits at the pubkey stored in the spot fixture
///   (SpotMarket.oracle @ +40);
/// * all accounts owned by the Velocity program id (`owner == id`).
///
/// Returns the six addresses for the OnPriceTick remaining tail in
/// OracleMap/SpotMarketMap/PerpMarketMap load order (oracles first, then spot
/// market, then perp market).
fn set_drift_accounts(
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

    let mk = |data: Vec<u8>, _addr: Address| Account {
        lamports: 1_000_000_000,
        data,
        owner: drift_pubkey,
        executable: false,
        rent_epoch: 0,
    };

    svm.set_account(
        state_addr,
        mk(read_fixture("velocity_state.bin"), state_addr),
    )
    .unwrap();
    svm.set_account(
        perp_addr,
        mk(read_fixture("velocity_perp_market_0.bin"), perp_addr),
    )
    .unwrap();
    svm.set_account(
        spot_addr,
        mk(read_fixture("velocity_spot_market_0.bin"), spot_addr),
    )
    .unwrap();
    svm.set_account(
        sol_oracle_addr,
        mk(read_fixture("velocity_oracle_perp.bin"), sol_oracle_addr),
    )
    .unwrap();
    svm.set_account(
        usdc_oracle_addr,
        mk(read_fixture("velocity_oracle_spot.bin"), usdc_oracle_addr),
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
fn autonomous_tick_places_reduce_with_real_velocity_program() {
    let program_id = Address::from_str_const(PROGRAM_ID);
    let drift_id = Address::from_str_const(DRIFT_PROGRAM_ID);
    let clock = Address::from_str_const(CLOCK_SYSVAR);
    let rent = Address::from_str_const(RENT_SYSVAR);
    let system = Address::from_str_const(SYSTEM_PROGRAM);

    let mut svm = LiteSVM::new().with_sigverify(false);
    svm.add_program(program_id, &read_so("target/deploy/wick_guard.so"))
        .unwrap();
    // REAL Velocity program — decompressed from the vendored gzip fixture.
    svm.add_program(drift_id, &decompress_velocity_so())
        .unwrap();

    let owner_kp = Keypair::new();
    let owner = owner_kp.pubkey();
    svm.airdrop(&owner, 100 * LAMPORTS_PER_SOL).unwrap();

    let seeds: &[&[u8]] = &[GUARD_SEED, owner.as_ref()];
    let (guard_pda, bump) = Address::find_program_address(seeds, &program_id);

    // --- InitGuard (venue = velocity, Autonomous) ---
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

    // --- Real Velocity accounts (oracles at their real mainnet pubkeys) ---
    let drift_pubkey = Pubkey::from(drift_id.to_bytes());
    let (state, user, oracle_sol, oracle_usdc, spot, perp) =
        set_drift_accounts(&mut svm, drift_pubkey, guard_pda);

    // --- Warp the clock so Velocity's markets are freshly posted. ---
    svm.warp_to_slot(TICK_SLOT);

    #[allow(clippy::too_many_arguments)]
    fn build_tick(
        program_id: Address,
        guard_pda: Address,
        clock: Address,
        state: Address,
        user: Address,
        oracle_sol: Address,
        oracle_usdc: Address,
        spot: Address,
        perp: Address,
        drift_id: Address,
        bump: u8,
    ) -> Instruction {
        let mut data = vec![7u8];
        data.extend_from_slice(&50_000_000u128.to_le_bytes());
        data.extend_from_slice(&1u64.to_le_bytes());
        data.push(bump);
        Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(guard_pda, false),            // [0] guard
                AccountMeta::new_readonly(clock, false),       // [1] clock
                AccountMeta::new_readonly(state, false),       // [2] Real State
                AccountMeta::new(user, false),                 // [3] Real User
                AccountMeta::new(guard_pda, false),            // [4] authority = guard PDA
                AccountMeta::new_readonly(oracle_sol, false),  // [5] perp oracle
                AccountMeta::new_readonly(oracle_usdc, false), // [6] spot oracle
                AccountMeta::new_readonly(spot, false),        // [7] spot market 0
                AccountMeta::new_readonly(perp, false),        // [8] perp market 0
                AccountMeta::new_readonly(drift_id, false),    // CPI target
            ],
            data,
        }
    }

    // Warmup tick at TICK_SLOT: guard cold-start (last_check_slot=0) so this is
    // judged stale and only rolls in the freshness anchor.
    let warm_ix = build_tick(
        program_id,
        guard_pda,
        clock,
        state,
        user,
        oracle_sol,
        oracle_usdc,
        spot,
        perp,
        drift_id,
        bump,
    );
    svm.send_transaction(Transaction::new(
        &[&owner_kp],
        Message::new_with_blockhash(&[warm_ix], Some(&owner), &svm.latest_blockhash()),
        svm.latest_blockhash(),
    ))
    .expect("warmup tick failed");

    // Dispatch tick: last_check_slot == TICK_SLOT ⇒ fresh; breach ⇒
    // TopUp→cap→PartialClose ⇒ autonomous real-velocity reduce.
    let tick = build_tick(
        program_id,
        guard_pda,
        clock,
        state,
        user,
        oracle_sol,
        oracle_usdc,
        spot,
        perp,
        drift_id,
        bump,
    );
    let tx = Transaction::new(
        &[&owner_kp],
        Message::new_with_blockhash(&[tick], Some(&owner), &svm.latest_blockhash()),
        svm.latest_blockhash(),
    );
    let res = svm.send_transaction(tx);
    assert!(res.is_ok(), "OnPriceTick failed: {res:?}");

    // --- Assertions ---
    // The guard's nonce committed ⇒ the autonomous CPI landed (§8.4).
    let guard_after = svm.get_account(&guard_pda).expect("guard missing");
    let nonce = u64::from_le_bytes(guard_after.data[243..251].try_into().unwrap());
    assert_eq!(nonce, 1, "nonce must commit on a landed venue action");

    // Real Velocity recorded the open order: next_order_id bumped 1 → 2, and
    // orders[0] is the reduce (reduce_only boot at Order offset 92; the reduce
    // of a long is a Short (direction=1, offset 91); status Open (1, offset 86)).
    let user_after = svm.get_account(&user).expect("user missing");
    let next_id = u32::from_le_bytes(
        user_after.data[NEXT_ORDER_ID_OFFSET..NEXT_ORDER_ID_OFFSET + 4]
            .try_into()
            .unwrap(),
    );
    assert_eq!(next_id, 2, "real velocity must record the reduce order");
    let order0 = ORDERS_OFFSET;
    assert_eq!(user_after.data[order0 + 86], 1, "order must be Open");
    assert_eq!(
        user_after.data[order0 + 91],
        1,
        "reduce of a long must be Short"
    );
    assert_eq!(user_after.data[order0 + 92], 1, "order must be reduce-only");
}

fn read_so(rel: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push(rel);
    std::fs::read(&p).unwrap_or_else(|e| {
        panic!("{p:?} not found ({e}). Run `cargo build-sbf` in program/ first.")
    })
}
