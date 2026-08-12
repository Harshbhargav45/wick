//! Wire-format serialization for Wick guard accounts.
//!
//! We deliberately do NOT cast `repr(C)` structs onto account data: struct
//! padding and enum sizes are implementation-defined, and this is a `no_std`
//! program running in the BPF sandbox where determinism matters. Instead every
//! account layout is a fixed byte map written/read with explicit offsets.
//!
//! All integers are little-endian.

// The decode/encode helpers return `Result<_, ()>` where `()` is a deliberate
// sentinel for "not our data / short buffer". Callers map it to
// `WickError::NotInitialized`. A dedicated error enum here would add surface
// for an unconditional `()` signal, so the unit error type is kept.
#![allow(clippy::result_unit_err)]

use crate::state::{Action, ActionCaps, AuthorityRequirement, RouteConfig, VenuePolicy};

/// Account data version marker. Anything else is `NotInitialized`/foreign.
///
/// v2 added the daily-spend accumulator (`daily_spent_usd`,
/// `daily_epoch_start_ts`) to `PositionGuard`, extending `GUARD_DATA_LEN`
/// from 342 to 366. There is no migration path from v1: `from_bytes` rejects
/// the old length and version outright, so v1 guards are orphaned and must be
/// re-initialized. This is deliberate — v1 only ever ran on devnet, and the
/// alternative (a realloc/upgrade instruction) is unaudited surface protecting
/// accounts that hold nothing.
///
/// v3 appends the venue-reconciliation block (`venue_size`,
/// `venue_collateral`, `reconcile_ts`, `reconcile_nonce`, `reconcile_status`)
/// and `margin_wallet_bump`, extending `GUARD_DATA_LEN` from 366 to 416.
/// The v1→v2 precedent holds: there is no migration. A v2 guard fails
/// `from_bytes` on both length and version, and the escape hatch is
/// `CloseGuard` (ix 11) — it refunds the rent and zeroes the account so the
/// owner can re-init at the same PDA, which is a pure function of the owner.
pub const ACCOUNT_VERSION: u8 = 3;

// -------------------------------------------------------------------------
// GuardMarginWallet  — a thin 2-of-2 margin reserve
// Layout (bytes): [0]=version, [1..33]=owner, [33..65]=co_authority,
//                  [65..81]=balance (u128 LE)
// -------------------------------------------------------------------------
pub const WALLET_DATA_LEN: usize = 81;
const WALLET_OWNER_OFF: usize = 1;
const WALLET_CO_OFF: usize = 33;
const WALLET_BALANCE_OFF: usize = 65;

/// The on-chain margin-wallet state, decoded.
pub struct WalletState {
    pub owner: [u8; 32],
    pub co_authority: [u8; 32],
    pub balance: u128,
}

impl WalletState {
    /// Decode from a full account-data slice. Returns `Err` if the slice is
    /// not `WALLET_DATA_LEN` or its version marker mismatches.
    pub fn from_bytes(data: &[u8]) -> Result<Self, ()> {
        if data.len() != WALLET_DATA_LEN || data[0] != ACCOUNT_VERSION {
            return Err(());
        }
        let mut owner = [0u8; 32];
        owner.copy_from_slice(&data[WALLET_OWNER_OFF..WALLET_CO_OFF]);
        let mut co_authority = [0u8; 32];
        co_authority.copy_from_slice(&data[WALLET_CO_OFF..WALLET_BALANCE_OFF]);
        let balance = u128::from_le_bytes(
            data[WALLET_BALANCE_OFF..WALLET_BALANCE_OFF + 16]
                .try_into()
                .map_err(|_| ())?,
        );
        Ok(Self {
            owner,
            co_authority,
            balance,
        })
    }

    /// Serialize into an existing account-data buffer. Returns `Err` if the
    /// buffer is too small.
    pub fn write_into(&self, out: &mut [u8]) -> Result<(), ()> {
        if out.len() < WALLET_DATA_LEN {
            return Err(());
        }
        out[0] = ACCOUNT_VERSION;
        out[WALLET_OWNER_OFF..WALLET_CO_OFF].copy_from_slice(&self.owner);
        out[WALLET_CO_OFF..WALLET_BALANCE_OFF].copy_from_slice(&self.co_authority);
        out[WALLET_BALANCE_OFF..WALLET_BALANCE_OFF + 16]
            .copy_from_slice(&self.balance.to_le_bytes());
        Ok(())
    }
}

// -------------------------------------------------------------------------
// RouteConfig  — singleton program config (kill-switch + pause)
// Layout: [0]=version, [1..33]=authority, [33]=paused(u8)
// -------------------------------------------------------------------------
pub const ROUTE_CONFIG_LEN: usize = 34;
const ROUTE_AUTHORITY_OFF: usize = 1;
const ROUTE_PAUSED_OFF: usize = 33;

impl RouteConfig {
    pub fn from_bytes(data: &[u8]) -> Result<Self, ()> {
        if data.len() != ROUTE_CONFIG_LEN || data[0] != ACCOUNT_VERSION {
            return Err(());
        }
        let mut authority = [0u8; 32];
        authority.copy_from_slice(&data[ROUTE_AUTHORITY_OFF..ROUTE_PAUSED_OFF]);
        Ok(Self {
            authority,
            paused: data[ROUTE_PAUSED_OFF] == 1,
            _padding: [0u8; 31],
        })
    }

    pub fn write_into(&self, out: &mut [u8]) -> Result<(), ()> {
        if out.len() < ROUTE_CONFIG_LEN {
            return Err(());
        }
        out[0] = ACCOUNT_VERSION;
        out[ROUTE_AUTHORITY_OFF..ROUTE_PAUSED_OFF].copy_from_slice(&self.authority);
        out[ROUTE_PAUSED_OFF] = self.paused as u8;
        Ok(())
    }
}

// -------------------------------------------------------------------------
// PositionGuard  — watched position + policy + live health snapshot
// Layout (bytes):
//   [0]        version
//   [1]        venue (u8)
//   [2..34]    venue_owner
//   [34..66]   co_authority
//   [66]       authority_req (u8: 0=Autonomous, 1=CoSigned)
//   [67..83]   maintenance_bps  (u128)
//   [83..99]   trigger_buffer_bps (u128)
//   [99..115]  fee_bps (u128)
//   [115..131] cap_top_up (u128)
//   [131..147] cap_partial_close (u128)
//   [147..163] cap_daily (u128)
//   [163..179] take_profit (u128)   (U128_MAX = none)
//   [179..195] collateral (u128)
//   [195..211] size (i128)
//   [211..227] entry (u128)
//   [227..243] current_price (u128)
//   [243..251] nonce (u64)
//   [251..259] last_check_ts (i64 LE) — unix seconds of the last accepted tick
//   [259]      pending tag (u8, 0=None,1=TopUp,2=PartialClose,3=TakeProfit,4=Escalate)
//   [260..276] pending amount (u128, meaningful for TopUp/PartialClose)
//   [276]      degraded (u8: 0=healthy, 1=degraded — §8.1.3)
//   [277]      stale streak (u8 — consecutive stale ticks)
//   [278]      pending_ix tag (u8, 0=None, 1=JupiterInstantTpsl)
//   [279..287] pending_ix expected_nonce (u64)
//   [287..337] pending_ix data (50 bytes — owner-signed instruction built by
//              the guard; see jupiter.rs, build-only boundary)
//   [338..340] drift_market_index (u16 LE) — perp market the guard reduces (§8.7)
//   [340..342] drift_subaccount_id (u16 LE) — Drift sub-account delegated to
//              the guard (Drift user PDA seed `sub_account_id`)
//   [342..358] daily_spent_usd (u128 LE) — USD committed in the current epoch
//   [358..366] daily_epoch_start_ts (i64 LE) — unix seconds the epoch began;
//              the accumulator resets when it is older than DAILY_EPOCH_SECS
//   [366..382] venue_size (i128 LE) — venue-reported size at the last reconcile
//   [382..398] venue_collateral (u128 LE, 6dp) — venue-reported collateral
//   [398..406] reconcile_ts (i64 LE) — unix seconds of the last reconcile
//   [406..414] reconcile_nonce (u64 LE) — monotonic; kills stale/duplicate
//   [414]      reconcile_status (u8: 0=NeverReconciled, 1=Converged, 2=Diverged)
//   [415]      margin_wallet_bump (u8) — 0 = no wallet linked
// Total: 416
// -------------------------------------------------------------------------
pub const GUARD_DATA_LEN: usize = 416;

/// Max size of a guard-built owner-signed instruction payload (discriminator +
/// `InstantCreateTpslParams`). Must match `jupiter::build_instant_tpsl_data`.
pub const PENDING_IX_DATA_LEN: usize = 50;

const G_VENUE_OFF: usize = 1;
const G_VENUE_OWNER_OFF: usize = 2;
const G_CO_AUTH_OFF: usize = 34;
const G_AUTH_REQ_OFF: usize = 66;
const G_MAINT_OFF: usize = 67;
const G_BUF_OFF: usize = 83;
const G_FEE_OFF: usize = 99;
const G_CAP_TOP_OFF: usize = 115;
const G_CAP_PARTIAL_OFF: usize = 131;
const G_CAP_DAILY_OFF: usize = 147;
const G_TP_OFF: usize = 163;
const G_COLLAT_OFF: usize = 179;
const G_SIZE_OFF: usize = 195;
const G_ENTRY_OFF: usize = 211;
const G_PRICE_OFF: usize = 227;
const G_NONCE_OFF: usize = 243;
const G_SLOT_OFF: usize = 251;
const G_PENDING_TAG_OFF: usize = 259;
const G_PENDING_AMT_OFF: usize = 260;
const G_DEGRADED_OFF: usize = 276;
const G_STALE_STREAK_OFF: usize = 277;
const G_PX_TAG_OFF: usize = 278;
const G_PX_NONCE_OFF: usize = 279;
const G_PX_DATA_OFF: usize = 287;
const G_DRIFT_MARKET_OFF: usize = 338;
const G_DRIFT_SUBACCOUNT_OFF: usize = 340;
const G_DAILY_SPENT_OFF: usize = 342;
const G_DAILY_EPOCH_OFF: usize = 358;
const G_RECON_VENUE_SIZE_OFF: usize = 366;
const G_RECON_VENUE_COLLAT_OFF: usize = 382;
const G_RECON_TS_OFF: usize = 398;
const G_RECON_NONCE_OFF: usize = 406;
const G_RECON_STATUS_OFF: usize = 414;
const G_MARGIN_WALLET_BUMP_OFF: usize = 415;

/// `reconcile_status` values ([414]). `Diverged` is a fail-closed state: the
/// last reconcile observed the venue position out of tolerance, so autonomous
/// execution is blocked until a fresh reconcile converges or the owner
/// intervenes (§8.3.4).
pub const RECONCILE_NEVER: u8 = 0;
pub const RECONCILE_CONVERGED: u8 = 1;
pub const RECONCILE_DIVERGED: u8 = 2;

const NONE_PRICE: u128 = u128::MAX;

/// `pending_ix` tag values ([278]). The tag is not decoration: `confirm_pending`
/// only ever lands tag 1, and the console must not offer a confirm button for
/// anything else (see `jupiter::build_defensive_close`).
pub const PENDING_IX_NONE: u8 = 0;
pub const PENDING_IX_JUPITER_TPSL: u8 = 1;
pub const PENDING_IX_JUPITER_DEFENSIVE_CLOSE: u8 = 2;

/// A guard-built owner-signed instruction awaiting the position owner's
/// signature (§8.4 CoSigned). Build-only: the guard constructs the serialized
/// payload and records the nonce it expects to commit when the owner lands it;
/// it never signs or submits the instruction itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingIx {
    /// Which builder produced `data` — `PENDING_IX_JUPITER_TPSL` or
    /// `PENDING_IX_JUPITER_DEFENSIVE_CLOSE`. Two different instructions with
    /// identical byte lengths are indistinguishable without it, and confirming
    /// the wrong one would land a take-profit where a stop was intended.
    pub kind: u8,
    /// The nonce the guard will commit iff the owner confirms the build.
    pub expected_nonce: u64,
    /// Serialized owner-signed instruction data (see `jupiter::build_instant_tpsl_data`).
    pub data: [u8; PENDING_IX_DATA_LEN],
}

/// Decoded view of a `PositionGuard` account.
pub struct GuardState {
    pub venue: u8,
    pub venue_owner: [u8; 32],
    pub co_authority: [u8; 32],
    pub authority_req: AuthorityRequirement,
    pub policy: VenuePolicy,
    pub collateral: u128,
    pub size: i128,
    pub entry: u128,
    pub current_price: u128,
    pub nonce: u64,
    pub last_check_ts: i64,
    pub pending: Option<Action>,
    pub pending_ix: Option<PendingIx>,
    pub degraded: bool,
    pub stale_streak: u8,
    /// Drift perp market the guard watches/reduces (only for `venue = VENUE_DRIFT`).
    pub drift_market_index: u16,
    /// Drift sub-account whose delegate is the guard PDA (only for `venue = VENUE_DRIFT`).
    pub drift_subaccount_id: u16,
    /// USD (6dp) already committed by guard actions in the current daily epoch.
    /// Checked and incremented on commit; see `state::ActionCaps::within_daily`.
    pub daily_spent_usd: u128,
    /// Unix timestamp (seconds) the current daily epoch began. Once `now` is
    /// more than `DAILY_EPOCH_SECS` past this, the accumulator rolls over.
    pub daily_epoch_start_ts: i64,
    /// Venue-reported position size at the last reconcile (§8.3). Only
    /// meaningful once `reconcile_status != RECONCILE_NEVER`.
    pub venue_size: i128,
    /// Venue-reported collateral (6dp) at the last reconcile.
    pub venue_collateral: u128,
    /// Unix seconds of the last successful reconcile; 0 if never.
    pub reconcile_ts: i64,
    /// Monotonic reconcile nonce. `ReconcileVenue` demands strictly increasing
    /// values, which is what makes the permissionless reconcile call replay-safe.
    pub reconcile_nonce: u64,
    /// One of `RECONCILE_NEVER` / `RECONCILE_CONVERGED` / `RECONCILE_DIVERGED`.
    pub reconcile_status: u8,
    /// PDA bump of the linked margin wallet (`b"margin" || venue_owner`); 0
    /// means no wallet is linked (a v2-era guard that predates the wallet).
    pub margin_wallet_bump: u8,
}

impl GuardState {
    pub fn from_bytes(data: &[u8]) -> Result<Self, ()> {
        if data.len() != GUARD_DATA_LEN || data[0] != ACCOUNT_VERSION {
            return Err(());
        }
        let rd = |off: usize| -> Result<u128, ()> {
            Ok(u128::from_le_bytes(
                data[off..off + 16].try_into().map_err(|_| ())?,
            ))
        };
        let mut venue_owner = [0u8; 32];
        venue_owner.copy_from_slice(&data[G_VENUE_OWNER_OFF..G_CO_AUTH_OFF]);
        let mut co_authority = [0u8; 32];
        co_authority.copy_from_slice(&data[G_CO_AUTH_OFF..G_AUTH_REQ_OFF]);

        let tp_raw = rd(G_TP_OFF)?;
        let pending = match data[G_PENDING_TAG_OFF] {
            0 => None,
            1 => Some(Action::TopUp {
                amount: rd(G_PENDING_AMT_OFF)?,
            }),
            2 => Some(Action::PartialClose {
                fraction_bps: rd(G_PENDING_AMT_OFF)?,
            }),
            3 => Some(Action::TakeProfit),
            4 => Some(Action::EscalateManualReview),
            _ => return Err(()),
        };

        let pending_ix = match data[G_PX_TAG_OFF] {
            PENDING_IX_NONE => None,
            kind @ (PENDING_IX_JUPITER_TPSL | PENDING_IX_JUPITER_DEFENSIVE_CLOSE) => {
                let expected_nonce = u64::from_le_bytes(
                    data[G_PX_NONCE_OFF..G_PX_NONCE_OFF + 8]
                        .try_into()
                        .map_err(|_| ())?,
                );
                let mut px = [0u8; PENDING_IX_DATA_LEN];
                px.copy_from_slice(&data[G_PX_DATA_OFF..G_PX_DATA_OFF + PENDING_IX_DATA_LEN]);
                Some(PendingIx {
                    kind,
                    expected_nonce,
                    data: px,
                })
            }
            _ => return Err(()),
        };

        let reconcile_status = data[G_RECON_STATUS_OFF];
        if reconcile_status > RECONCILE_DIVERGED {
            return Err(());
        }

        Ok(Self {
            venue: data[G_VENUE_OFF],
            venue_owner,
            co_authority,
            authority_req: if data[G_AUTH_REQ_OFF] == 0 {
                AuthorityRequirement::Autonomous
            } else {
                AuthorityRequirement::CoSigned
            },
            policy: VenuePolicy {
                maintenance_bps: rd(G_MAINT_OFF)?,
                trigger_buffer_bps: rd(G_BUF_OFF)?,
                fee_bps: rd(G_FEE_OFF)?,
                authority: if data[G_AUTH_REQ_OFF] == 0 {
                    AuthorityRequirement::Autonomous
                } else {
                    AuthorityRequirement::CoSigned
                },
                caps: ActionCaps {
                    top_up_usd_per_action: rd(G_CAP_TOP_OFF)?,
                    partial_close_usd_per_action: rd(G_CAP_PARTIAL_OFF)?,
                    daily_total_usd: rd(G_CAP_DAILY_OFF)?,
                },
                take_profit: if tp_raw == NONE_PRICE {
                    None
                } else {
                    Some(tp_raw)
                },
            },
            collateral: rd(G_COLLAT_OFF)?,
            size: i128::from_le_bytes(
                data[G_SIZE_OFF..G_SIZE_OFF + 16]
                    .try_into()
                    .map_err(|_| ())?,
            ),
            entry: rd(G_ENTRY_OFF)?,
            current_price: rd(G_PRICE_OFF)?,
            nonce: u64::from_le_bytes(
                data[G_NONCE_OFF..G_NONCE_OFF + 8]
                    .try_into()
                    .map_err(|_| ())?,
            ),
            last_check_ts: i64::from_le_bytes(
                data[G_SLOT_OFF..G_SLOT_OFF + 8]
                    .try_into()
                    .map_err(|_| ())?,
            ),
            pending,
            pending_ix,
            degraded: data[G_DEGRADED_OFF] == 1,
            stale_streak: data[G_STALE_STREAK_OFF],
            drift_market_index: u16::from_le_bytes(
                data[G_DRIFT_MARKET_OFF..G_DRIFT_MARKET_OFF + 2]
                    .try_into()
                    .map_err(|_| ())?,
            ),
            drift_subaccount_id: u16::from_le_bytes(
                data[G_DRIFT_SUBACCOUNT_OFF..G_DRIFT_SUBACCOUNT_OFF + 2]
                    .try_into()
                    .map_err(|_| ())?,
            ),
            daily_spent_usd: rd(G_DAILY_SPENT_OFF)?,
            daily_epoch_start_ts: i64::from_le_bytes(
                data[G_DAILY_EPOCH_OFF..G_DAILY_EPOCH_OFF + 8]
                    .try_into()
                    .map_err(|_| ())?,
            ),
            venue_size: i128::from_le_bytes(
                data[G_RECON_VENUE_SIZE_OFF..G_RECON_VENUE_SIZE_OFF + 16]
                    .try_into()
                    .map_err(|_| ())?,
            ),
            venue_collateral: rd(G_RECON_VENUE_COLLAT_OFF)?,
            reconcile_ts: i64::from_le_bytes(
                data[G_RECON_TS_OFF..G_RECON_TS_OFF + 8]
                    .try_into()
                    .map_err(|_| ())?,
            ),
            reconcile_nonce: u64::from_le_bytes(
                data[G_RECON_NONCE_OFF..G_RECON_NONCE_OFF + 8]
                    .try_into()
                    .map_err(|_| ())?,
            ),
            reconcile_status,
            margin_wallet_bump: data[G_MARGIN_WALLET_BUMP_OFF],
        })
    }

    /// Write into an existing buffer. `pending` must be normalized to the
    /// tag+amount encoding by the caller.
    pub fn write_into(&self, out: &mut [u8]) -> Result<(), ()> {
        if out.len() < GUARD_DATA_LEN {
            return Err(());
        }
        fn wr(out: &mut [u8], off: usize, v: u128) {
            out[off..off + 16].copy_from_slice(&v.to_le_bytes());
        }
        let (tag, amt) = match self.pending {
            None => (0u8, 0u128),
            Some(Action::TopUp { amount }) => (1, amount),
            Some(Action::PartialClose { fraction_bps }) => (2, fraction_bps),
            Some(Action::TakeProfit) => (3, 0),
            Some(Action::EscalateManualReview) => (4, 0),
        };

        out[0] = ACCOUNT_VERSION;
        out[G_VENUE_OFF] = self.venue;
        out[G_VENUE_OWNER_OFF..G_CO_AUTH_OFF].copy_from_slice(&self.venue_owner);
        out[G_CO_AUTH_OFF..G_AUTH_REQ_OFF].copy_from_slice(&self.co_authority);
        out[G_AUTH_REQ_OFF] = match self.authority_req {
            AuthorityRequirement::Autonomous => 0,
            AuthorityRequirement::CoSigned => 1,
        };
        wr(out, G_MAINT_OFF, self.policy.maintenance_bps);
        wr(out, G_BUF_OFF, self.policy.trigger_buffer_bps);
        wr(out, G_FEE_OFF, self.policy.fee_bps);
        wr(out, G_CAP_TOP_OFF, self.policy.caps.top_up_usd_per_action);
        wr(
            out,
            G_CAP_PARTIAL_OFF,
            self.policy.caps.partial_close_usd_per_action,
        );
        wr(out, G_CAP_DAILY_OFF, self.policy.caps.daily_total_usd);
        match self.policy.take_profit {
            Some(tp) => wr(out, G_TP_OFF, tp),
            None => wr(out, G_TP_OFF, NONE_PRICE),
        }
        wr(out, G_COLLAT_OFF, self.collateral);
        out[G_SIZE_OFF..G_SIZE_OFF + 16].copy_from_slice(&self.size.to_le_bytes());
        wr(out, G_ENTRY_OFF, self.entry);
        wr(out, G_PRICE_OFF, self.current_price);
        out[G_NONCE_OFF..G_NONCE_OFF + 8].copy_from_slice(&self.nonce.to_le_bytes());
        out[G_SLOT_OFF..G_SLOT_OFF + 8].copy_from_slice(&self.last_check_ts.to_le_bytes());
        out[G_PENDING_TAG_OFF] = tag;
        wr(out, G_PENDING_AMT_OFF, amt);
        out[G_DEGRADED_OFF] = if self.degraded { 1 } else { 0 };
        out[G_STALE_STREAK_OFF] = self.stale_streak;
        out[G_DRIFT_MARKET_OFF..G_DRIFT_MARKET_OFF + 2]
            .copy_from_slice(&self.drift_market_index.to_le_bytes());
        out[G_DRIFT_SUBACCOUNT_OFF..G_DRIFT_SUBACCOUNT_OFF + 2]
            .copy_from_slice(&self.drift_subaccount_id.to_le_bytes());
        wr(out, G_DAILY_SPENT_OFF, self.daily_spent_usd);
        out[G_DAILY_EPOCH_OFF..G_DAILY_EPOCH_OFF + 8]
            .copy_from_slice(&self.daily_epoch_start_ts.to_le_bytes());
        out[G_RECON_VENUE_SIZE_OFF..G_RECON_VENUE_SIZE_OFF + 16]
            .copy_from_slice(&self.venue_size.to_le_bytes());
        wr(out, G_RECON_VENUE_COLLAT_OFF, self.venue_collateral);
        out[G_RECON_TS_OFF..G_RECON_TS_OFF + 8].copy_from_slice(&self.reconcile_ts.to_le_bytes());
        out[G_RECON_NONCE_OFF..G_RECON_NONCE_OFF + 8]
            .copy_from_slice(&self.reconcile_nonce.to_le_bytes());
        out[G_RECON_STATUS_OFF] = self.reconcile_status;
        out[G_MARGIN_WALLET_BUMP_OFF] = self.margin_wallet_bump;
        match self.pending_ix {
            None => out[G_PX_TAG_OFF] = PENDING_IX_NONE,
            Some(px) => {
                out[G_PX_TAG_OFF] = px.kind;
                out[G_PX_NONCE_OFF..G_PX_NONCE_OFF + 8]
                    .copy_from_slice(&px.expected_nonce.to_le_bytes());
                out[G_PX_DATA_OFF..G_PX_DATA_OFF + PENDING_IX_DATA_LEN].copy_from_slice(&px.data);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Action, ActionCaps, AuthorityRequirement, VenuePolicy};

    #[test]
    fn wallet_roundtrip() {
        let mut buf = [0u8; WALLET_DATA_LEN];
        let w = WalletState {
            owner: [7u8; 32],
            co_authority: [9u8; 32],
            balance: 123_456_789,
        };
        w.write_into(&mut buf).unwrap();
        let back = WalletState::from_bytes(&buf).unwrap();
        assert_eq!(back.owner, [7u8; 32]);
        assert_eq!(back.co_authority, [9u8; 32]);
        assert_eq!(back.balance, 123_456_789);
    }

    #[test]
    fn wallet_rejects_foreign_version() {
        let buf = [0u8; WALLET_DATA_LEN];
        assert!(WalletState::from_bytes(&buf).is_err());
    }

    #[test]
    fn guard_roundtrip_all_fields() {
        let mut buf = [0u8; GUARD_DATA_LEN];
        let g = GuardState {
            venue: 1,
            venue_owner: [1u8; 32],
            co_authority: [2u8; 32],
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
            size: -250_000_000,
            entry: 50_000_000,
            current_price: 61_000_000,
            nonce: 42,
            last_check_ts: 9000,
            pending: Some(Action::TopUp { amount: 7 }),
            pending_ix: Some(PendingIx {
                kind: PENDING_IX_JUPITER_TPSL,
                expected_nonce: 43,
                data: [7u8; PENDING_IX_DATA_LEN],
            }),
            degraded: true,
            stale_streak: 3,
            drift_market_index: 7,
            drift_subaccount_id: 2,
            daily_spent_usd: 0,
            daily_epoch_start_ts: 0,
            venue_size: -249_000_000,
            venue_collateral: 99_500_000,
            reconcile_ts: 8_900,
            reconcile_nonce: 11,
            reconcile_status: RECONCILE_CONVERGED,
            margin_wallet_bump: 254,
        };
        g.write_into(&mut buf).unwrap();
        let back = GuardState::from_bytes(&buf).unwrap();
        assert_eq!(back.venue, 1);
        assert_eq!(back.venue_owner, [1u8; 32]);
        assert_eq!(back.co_authority, [2u8; 32]);
        assert_eq!(back.authority_req, AuthorityRequirement::CoSigned);
        assert_eq!(back.policy.maintenance_bps, 500);
        assert_eq!(back.policy.caps.daily_total_usd, 5_000);
        assert_eq!(back.policy.take_profit, Some(60_000_000));
        assert_eq!(back.collateral, 100_000_000);
        assert_eq!(back.size, -250_000_000);
        assert_eq!(back.entry, 50_000_000);
        assert_eq!(back.current_price, 61_000_000);
        assert_eq!(back.nonce, 42);
        assert_eq!(back.last_check_ts, 9000);
        assert_eq!(back.pending, Some(Action::TopUp { amount: 7 }));
        assert_eq!(
            back.pending_ix,
            Some(PendingIx {
                kind: PENDING_IX_JUPITER_TPSL,
                expected_nonce: 43,
                data: [7u8; PENDING_IX_DATA_LEN],
            })
        );
        assert!(back.degraded);
        assert_eq!(back.stale_streak, 3);
        assert_eq!(back.drift_market_index, 7);
        assert_eq!(back.drift_subaccount_id, 2);
        assert_eq!(back.venue_size, -249_000_000);
        assert_eq!(back.venue_collateral, 99_500_000);
        assert_eq!(back.reconcile_ts, 8_900);
        assert_eq!(back.reconcile_nonce, 11);
        assert_eq!(back.reconcile_status, RECONCILE_CONVERGED);
        assert_eq!(back.margin_wallet_bump, 254);
    }

    /// A defensive close is a different instruction from a take-profit and has
    /// to survive the roundtrip as one — the tag is what `confirm_pending`
    /// keys on to refuse it.
    #[test]
    fn guard_defensive_close_pending_ix_roundtrip() {
        let mut buf = [0u8; GUARD_DATA_LEN];
        let mut g = sample_guard();
        g.pending_ix = Some(PendingIx {
            kind: PENDING_IX_JUPITER_DEFENSIVE_CLOSE,
            expected_nonce: 9,
            data: [3u8; PENDING_IX_DATA_LEN],
        });
        g.write_into(&mut buf).unwrap();
        assert_eq!(buf[G_PX_TAG_OFF], PENDING_IX_JUPITER_DEFENSIVE_CLOSE);
        let back = GuardState::from_bytes(&buf).unwrap();
        let px = back.pending_ix.unwrap();
        assert_eq!(px.kind, PENDING_IX_JUPITER_DEFENSIVE_CLOSE);
        assert_eq!(px.expected_nonce, 9);
        assert_eq!(px.data, [3u8; PENDING_IX_DATA_LEN]);
    }

    /// An out-of-range `reconcile_status` is corrupt data, not a status the
    /// program should guess at: decoding it as Converged would silently re-arm
    /// autonomous execution.
    #[test]
    fn guard_rejects_unknown_reconcile_status() {
        let mut buf = [0u8; GUARD_DATA_LEN];
        sample_guard().write_into(&mut buf).unwrap();
        buf[G_RECON_STATUS_OFF] = 3;
        assert!(GuardState::from_bytes(&buf).is_err());
    }

    #[test]
    fn guard_rejects_v2_length_and_version() {
        let mut buf = [0u8; GUARD_DATA_LEN];
        sample_guard().write_into(&mut buf).unwrap();
        // A v2 account: 366 bytes, version byte 2. Both must be rejected —
        // there is no migration, `CloseGuard` is the escape hatch.
        assert!(GuardState::from_bytes(&buf[..366]).is_err());
        buf[0] = 2;
        assert!(GuardState::from_bytes(&buf).is_err());
    }

    fn sample_guard() -> GuardState {
        GuardState {
            venue: 3,
            venue_owner: [1u8; 32],
            co_authority: [2u8; 32],
            authority_req: AuthorityRequirement::Autonomous,
            policy: VenuePolicy {
                maintenance_bps: 500,
                trigger_buffer_bps: 500,
                fee_bps: 10,
                authority: AuthorityRequirement::Autonomous,
                caps: ActionCaps {
                    top_up_usd_per_action: 1_000,
                    partial_close_usd_per_action: 2_000,
                    daily_total_usd: 5_000,
                },
                take_profit: None,
            },
            collateral: 100_000_000,
            size: 250_000_000,
            entry: 50_000_000,
            current_price: 51_000_000,
            nonce: 1,
            last_check_ts: 100,
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

    #[test]
    fn guard_none_take_profit_roundtrip() {
        let mut buf = [0u8; GUARD_DATA_LEN];
        let g = GuardState {
            venue: 0,
            venue_owner: [0u8; 32],
            co_authority: [0u8; 32],
            authority_req: AuthorityRequirement::Autonomous,
            policy: VenuePolicy {
                maintenance_bps: 500,
                trigger_buffer_bps: 0,
                fee_bps: 0,
                authority: AuthorityRequirement::Autonomous,
                caps: ActionCaps {
                    top_up_usd_per_action: 0,
                    partial_close_usd_per_action: 0,
                    daily_total_usd: 0,
                },
                take_profit: None,
            },
            collateral: 0,
            size: 0,
            entry: 0,
            current_price: 0,
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
        g.write_into(&mut buf).unwrap();
        let back = GuardState::from_bytes(&buf).unwrap();
        assert_eq!(back.policy.take_profit, None);
        assert_eq!(back.pending, None);
        assert_eq!(back.pending_ix, None);
        assert!(!back.degraded);
        assert_eq!(back.stale_streak, 0);
        assert_eq!(back.drift_market_index, 0);
        assert_eq!(back.drift_subaccount_id, 0);
        assert_eq!(back.reconcile_status, RECONCILE_NEVER);
        assert_eq!(back.margin_wallet_bump, 0);
    }

    #[test]
    fn route_config_roundtrip() {
        let mut buf = [0u8; ROUTE_CONFIG_LEN];
        let c = RouteConfig {
            authority: [3u8; 32],
            paused: true,
            _padding: [0u8; 31],
        };
        c.write_into(&mut buf).unwrap();
        let back = RouteConfig::from_bytes(&buf).unwrap();
        assert_eq!(back.authority, [3u8; 32]);
        assert!(back.paused);
    }
}
