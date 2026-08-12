//! §8.5 margin reserve against a real SBF VM.
//!
//! The unit tests in `processor::tests` cover derivation, authority and
//! accounting, but off-target a CPI is a no-op that returns `Ok`, so
//! `CreateAccount` and `Transfer` move nothing there. This exercises the part
//! only a real VM can prove: lamports actually arrive, the rent-backing
//! invariant holds against the live rent sysvar, and value leaves on two
//! signatures and no fewer.
//!
//! Requires `cargo build-sbf` first.

// litesvm's `FailedTransactionMetadata` carries the full transaction log, so it
// is a couple of hundred bytes. Boxing it would only obscure the assertions;
// this is a test binary, not a hot path.
#![allow(clippy::result_large_err)]

use litesvm::LiteSVM;
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_native_token::LAMPORTS_PER_SOL;
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::path::PathBuf;
use wick_guard::account::{ACCOUNT_VERSION, WALLET_DATA_LEN};

const PROGRAM_ID: &str = "US517G5965aydkZ46HS38QLi7UQiSojurfbQfKCELFx";
const RENT_SYSVAR: &str = "SysvarRent111111111111111111111111111111111";
const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";

const GUARD_SEED: &[u8] = b"guard";
const ROUTE_CONFIG_SEED: &[u8] = b"route_config";
const MARGIN_SEED: &[u8] = b"margin";

fn read_wick_program() -> Vec<u8> {
    let mut so_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    so_path.push("target/deploy/wick_guard.so");
    std::fs::read(&so_path).unwrap_or_else(|e| {
        panic!("wick_guard.so not found at {so_path:?} ({e}). Run `cargo build-sbf` first.")
    })
}

/// `InitGuard` payload with `co_authority` set to a key the test controls, so
/// the 2-of-2 withdraw can actually be signed.
fn init_data(bump: u8, co_authority: &Address) -> Vec<u8> {
    let mut data = vec![0u8, bump];
    let mut blob = vec![0u8; 150];
    blob[0] = 0; // venue = none
    blob[1..33].copy_from_slice(co_authority.as_ref());
    blob[33] = 1; // CoSigned
    blob[34..50].copy_from_slice(&500u128.to_le_bytes());
    blob[50..66].copy_from_slice(&500u128.to_le_bytes());
    blob[66..82].copy_from_slice(&10u128.to_le_bytes());
    blob[82..98].copy_from_slice(&u128::MAX.to_le_bytes());
    blob[98..114].copy_from_slice(&u128::MAX.to_le_bytes());
    blob[114..130].copy_from_slice(&u128::MAX.to_le_bytes());
    blob[130..146].copy_from_slice(&u128::MAX.to_le_bytes()); // no take_profit
    data.extend_from_slice(&blob);
    data
}

fn amount_data(disc: u8, lamports: u128) -> Vec<u8> {
    let mut d = vec![disc];
    d.extend_from_slice(&lamports.to_le_bytes());
    d
}

/// A guard + route config, already initialized, ready for margin-wallet work.
struct Fixture {
    svm: LiteSVM,
    program_id: Address,
    rent_sysvar: Address,
    owner_kp: Keypair,
    co_kp: Keypair,
    guard_pda: Address,
    route_config: Address,
    wallet_pda: Address,
    wallet_bump: u8,
}

impl Fixture {
    fn new() -> Self {
        let program_id = Address::from_str_const(PROGRAM_ID);
        let rent_sysvar = Address::from_str_const(RENT_SYSVAR);
        let system_program = Address::from_str_const(SYSTEM_PROGRAM);

        let mut svm = LiteSVM::new().with_sigverify(false);
        svm.add_program(program_id, &read_wick_program()).unwrap();

        let owner_kp = Keypair::new();
        let co_kp = Keypair::new();
        let owner = owner_kp.pubkey();
        svm.airdrop(&owner, 100 * LAMPORTS_PER_SOL).unwrap();

        let (guard_pda, guard_bump) =
            Address::find_program_address(&[GUARD_SEED, owner.as_ref()], &program_id);
        let (route_config, rc_bump) =
            Address::find_program_address(&[ROUTE_CONFIG_SEED], &program_id);
        let (wallet_pda, wallet_bump) =
            Address::find_program_address(&[MARGIN_SEED, owner.as_ref()], &program_id);

        let init_ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(guard_pda, false),
                AccountMeta::new_readonly(owner, true),
                AccountMeta::new(owner, true),
                AccountMeta::new_readonly(rent_sysvar, false),
                AccountMeta::new_readonly(system_program, false),
            ],
            data: init_data(guard_bump, &co_kp.pubkey()),
        };
        let rc_ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(route_config, false),
                AccountMeta::new_readonly(owner, true),
                AccountMeta::new(owner, true),
                AccountMeta::new_readonly(rent_sysvar, false),
                AccountMeta::new_readonly(system_program, false),
            ],
            data: vec![10u8, rc_bump],
        };
        let msg =
            Message::new_with_blockhash(&[init_ix, rc_ix], Some(&owner), &svm.latest_blockhash());
        svm.send_transaction(Transaction::new(&[&owner_kp], msg, svm.latest_blockhash()))
            .expect("guard + route config setup failed");

        Self {
            svm,
            program_id,
            rent_sysvar,
            owner_kp,
            co_kp,
            guard_pda,
            route_config,
            wallet_pda,
            wallet_bump,
        }
    }

    fn owner(&self) -> Address {
        self.owner_kp.pubkey()
    }

    fn send(
        &mut self,
        ix: Instruction,
        signers: &[&Keypair],
    ) -> Result<(), litesvm::types::FailedTransactionMetadata> {
        let payer = self.owner();
        let msg = Message::new_with_blockhash(&[ix], Some(&payer), &self.svm.latest_blockhash());
        let tx = Transaction::new(signers, msg, self.svm.latest_blockhash());
        self.svm.send_transaction(tx).map(|_| ())
    }

    fn init_wallet(&mut self) -> Result<(), litesvm::types::FailedTransactionMetadata> {
        let system_program = Address::from_str_const(SYSTEM_PROGRAM);
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.wallet_pda, false),
                AccountMeta::new(self.guard_pda, false),
                AccountMeta::new_readonly(self.owner(), true),
                AccountMeta::new(self.owner(), true),
                AccountMeta::new_readonly(self.rent_sysvar, false),
                AccountMeta::new_readonly(self.route_config, false),
                AccountMeta::new_readonly(system_program, false),
            ],
            data: vec![14u8, self.wallet_bump],
        };
        let owner_kp = self.owner_kp.insecure_clone();
        self.send(ix, &[&owner_kp])
    }

    fn fund(&mut self, lamports: u128) -> Result<(), litesvm::types::FailedTransactionMetadata> {
        let system_program = Address::from_str_const(SYSTEM_PROGRAM);
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.wallet_pda, false),
                AccountMeta::new_readonly(self.guard_pda, false),
                AccountMeta::new(self.owner(), true),
                AccountMeta::new_readonly(self.rent_sysvar, false),
                AccountMeta::new_readonly(self.route_config, false),
                AccountMeta::new_readonly(system_program, false),
            ],
            data: amount_data(15, lamports),
        };
        let owner_kp = self.owner_kp.insecure_clone();
        self.send(ix, &[&owner_kp])
    }

    /// `co_signs = false` drops the co-authority's signature flag, which is how
    /// the 2-of-2 rule gets tested rather than assumed.
    fn withdraw(
        &mut self,
        lamports: u128,
        co_signs: bool,
    ) -> Result<(), litesvm::types::FailedTransactionMetadata> {
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.wallet_pda, false),
                AccountMeta::new_readonly(self.guard_pda, false),
                AccountMeta::new(self.owner(), true),
                AccountMeta::new_readonly(self.co_kp.pubkey(), co_signs),
                AccountMeta::new_readonly(self.rent_sysvar, false),
                AccountMeta::new_readonly(self.route_config, false),
            ],
            data: amount_data(16, lamports),
        };
        let owner_kp = self.owner_kp.insecure_clone();
        let co_kp = self.co_kp.insecure_clone();
        if co_signs {
            self.send(ix, &[&owner_kp, &co_kp])
        } else {
            self.send(ix, &[&owner_kp])
        }
    }

    fn wallet_lamports(&self) -> u64 {
        self.svm
            .get_account(&self.wallet_pda)
            .expect("wallet missing")
            .lamports
    }

    fn wallet_balance(&self) -> u128 {
        let acc = self
            .svm
            .get_account(&self.wallet_pda)
            .expect("wallet missing");
        let mut b = [0u8; 16];
        b.copy_from_slice(&acc.data[65..81]);
        u128::from_le_bytes(b)
    }

    fn owner_lamports(&self) -> u64 {
        self.svm
            .get_account(&self.owner())
            .expect("owner missing")
            .lamports
    }

    /// Rent minimum straight from the VM's own sysvar, so the test and the
    /// program are reading the same number rather than a shared guess.
    fn rent_minimum(&self) -> u64 {
        self.svm.minimum_balance_for_rent_exemption(WALLET_DATA_LEN)
    }
}

/// Creation allocates a rent-exempt, program-owned account and links it to the
/// guard, with the guard's own 2-of-2 pair copied in.
#[test]
fn init_creates_a_rent_exempt_program_owned_reserve() {
    let mut fx = Fixture::new();
    fx.init_wallet().expect("InitMarginWallet failed");

    let wallet = fx.svm.get_account(&fx.wallet_pda).expect("wallet missing");
    assert_eq!(wallet.owner, fx.program_id);
    assert_eq!(wallet.data.len(), WALLET_DATA_LEN);
    assert_eq!(wallet.data[0], ACCOUNT_VERSION);
    assert_eq!(&wallet.data[1..33], fx.owner().as_ref());
    assert_eq!(&wallet.data[33..65], fx.co_kp.pubkey().as_ref());
    assert_eq!(fx.wallet_balance(), 0);
    assert!(
        wallet.lamports >= fx.rent_minimum(),
        "a reserve below its own rent would be reaped"
    );

    // And the guard now records the bump, which is what un-gates an autonomous
    // top-up in `on_price_tick`.
    let guard = fx.svm.get_account(&fx.guard_pda).expect("guard missing");
    assert_eq!(guard.data[415], fx.wallet_bump);
}

/// The whole point of the gap: lamports genuinely leave the owner and land in
/// the reserve, and `balance` records what arrived.
#[test]
fn funding_moves_real_lamports_and_credits_the_balance() {
    let mut fx = Fixture::new();
    fx.init_wallet().unwrap();
    let before_wallet = fx.wallet_lamports();
    let before_owner = fx.owner_lamports();

    fx.fund(2 * LAMPORTS_PER_SOL as u128)
        .expect("FundMarginWallet failed");

    assert_eq!(fx.wallet_lamports(), before_wallet + 2 * LAMPORTS_PER_SOL);
    assert_eq!(fx.wallet_balance(), 2 * LAMPORTS_PER_SOL as u128);
    assert!(
        fx.owner_lamports() <= before_owner - 2 * LAMPORTS_PER_SOL,
        "the lamports came from somewhere other than the owner"
    );

    // Two deposits accumulate rather than overwrite.
    fx.fund(LAMPORTS_PER_SOL as u128).unwrap();
    assert_eq!(fx.wallet_balance(), 3 * LAMPORTS_PER_SOL as u128);
    assert_eq!(fx.wallet_lamports(), before_wallet + 3 * LAMPORTS_PER_SOL);
}

/// A credit the owner cannot cover must fail whole — no lamports, no balance.
#[test]
fn funding_beyond_the_owners_means_fails_without_crediting() {
    let mut fx = Fixture::new();
    fx.init_wallet().unwrap();
    let before = fx.wallet_lamports();

    assert!(
        fx.fund(1_000 * LAMPORTS_PER_SOL as u128).is_err(),
        "a transfer the owner cannot fund must not succeed"
    );
    assert_eq!(
        fx.wallet_balance(),
        0,
        "balance credited against no transfer"
    );
    assert_eq!(fx.wallet_lamports(), before);
}

/// Value leaves on two signatures, and the lamports really arrive at the owner.
#[test]
fn withdraw_on_two_signatures_returns_real_lamports() {
    let mut fx = Fixture::new();
    fx.init_wallet().unwrap();
    fx.fund(3 * LAMPORTS_PER_SOL as u128).unwrap();
    let before_owner = fx.owner_lamports();
    let before_wallet = fx.wallet_lamports();

    fx.withdraw(LAMPORTS_PER_SOL as u128, true)
        .expect("WithdrawMarginWallet failed");

    assert_eq!(fx.wallet_lamports(), before_wallet - LAMPORTS_PER_SOL);
    assert_eq!(fx.wallet_balance(), 2 * LAMPORTS_PER_SOL as u128);
    assert!(fx.owner_lamports() > before_owner, "owner received nothing");
}

/// One signature is not enough, and the failure moves nothing.
#[test]
fn withdraw_without_the_co_authority_is_refused() {
    let mut fx = Fixture::new();
    fx.init_wallet().unwrap();
    fx.fund(2 * LAMPORTS_PER_SOL as u128).unwrap();
    let before_wallet = fx.wallet_lamports();
    let before_balance = fx.wallet_balance();

    assert!(fx.withdraw(LAMPORTS_PER_SOL as u128, false).is_err());
    assert_eq!(fx.wallet_lamports(), before_wallet);
    assert_eq!(fx.wallet_balance(), before_balance);
}

/// The rent-backing invariant under the real sysvar: the recorded balance is
/// withdrawable to the last lamport, and not one beyond it. Draining leaves the
/// account exactly rent-exempt rather than reapable.
#[test]
fn the_recorded_balance_is_exactly_what_is_withdrawable() {
    let mut fx = Fixture::new();
    fx.init_wallet().unwrap();
    fx.fund(2 * LAMPORTS_PER_SOL as u128).unwrap();

    assert!(
        fx.withdraw(2 * LAMPORTS_PER_SOL as u128 + 1, true).is_err(),
        "rent must not be withdrawable as if it were balance"
    );

    fx.withdraw(2 * LAMPORTS_PER_SOL as u128, true)
        .expect("draining the full balance must succeed");
    assert_eq!(fx.wallet_balance(), 0);
    assert_eq!(
        fx.wallet_lamports(),
        fx.rent_minimum(),
        "a drained reserve must still cover its own rent"
    );

    // And an empty reserve yields nothing further.
    assert!(fx.withdraw(1, true).is_err());
}

/// Re-initialization must not zero a funded balance.
#[test]
fn reinitializing_a_funded_reserve_is_refused() {
    let mut fx = Fixture::new();
    fx.init_wallet().unwrap();
    fx.fund(LAMPORTS_PER_SOL as u128).unwrap();

    assert!(fx.init_wallet().is_err(), "re-init must be rejected");
    assert_eq!(
        fx.wallet_balance(),
        LAMPORTS_PER_SOL as u128,
        "re-init wiped a funded balance"
    );
}
