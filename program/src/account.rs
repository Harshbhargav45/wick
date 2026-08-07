//! Wire-format serialization for Wick guard accounts (Phase 1).
//!
//! We deliberately do NOT cast `repr(C)` structs onto account data: struct
//! padding and enum sizes are implementation-defined, and this is a `no_std`
//! program running in the BPF sandbox where determinism matters. Instead every
//! account layout is a fixed byte map written/read with explicit offsets.
//!
//! All integers are little-endian.

use crate::state::{
    Action, ActionCaps, AuthorityRequirement, RouteConfig, VenuePolicy,
};

/// Account data version marker. Anything else is `NotInitialized`/foreign.
pub const ACCOUNT_VERSION: u8 = 1;

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
        Ok(Self { owner, co_authority, balance })
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
//   [251..259] last_check_slot (u64)
//   [259]      pending tag (u8, 0=None,1=TopUp,2=PartialClose,3=TakeProfit,4=Escalate)
//   [260..276] pending amount (u128, meaningful for TopUp/PartialClose)
// Total: 276
// -------------------------------------------------------------------------
pub const GUARD_DATA_LEN: usize = 276;

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

const NONE_PRICE: u128 = u128::MAX;

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
    pub last_check_slot: u64,
    pub pending: Option<Action>,
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
            1 => Some(Action::TopUp { amount: rd(G_PENDING_AMT_OFF)? }),
            2 => Some(Action::PartialClose { fraction_bps: rd(G_PENDING_AMT_OFF)? }),
            3 => Some(Action::TakeProfit),
            4 => Some(Action::EscalateManualReview),
            _ => return Err(()),
        };

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
                take_profit: if tp_raw == NONE_PRICE { None } else { Some(tp_raw) },
            },
            collateral: rd(G_COLLAT_OFF)?,
            size: i128::from_le_bytes(
                data[G_SIZE_OFF..G_SIZE_OFF + 16].try_into().map_err(|_| ())?,
            ),
            entry: rd(G_ENTRY_OFF)?,
            current_price: rd(G_PRICE_OFF)?,
            nonce: u64::from_le_bytes(
                data[G_NONCE_OFF..G_NONCE_OFF + 8].try_into().map_err(|_| ())?,
            ),
            last_check_slot: u64::from_le_bytes(
                data[G_SLOT_OFF..G_SLOT_OFF + 8].try_into().map_err(|_| ())?,
            ),
            pending,
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
        wr(out, G_CAP_PARTIAL_OFF, self.policy.caps.partial_close_usd_per_action);
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
        out[G_SLOT_OFF..G_SLOT_OFF + 8].copy_from_slice(&self.last_check_slot.to_le_bytes());
        out[G_PENDING_TAG_OFF] = tag;
        if amt != 0 {
            wr(out, G_PENDING_AMT_OFF, amt);
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
            last_check_slot: 9000,
            pending: Some(Action::TopUp { amount: 7 }),
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
        assert_eq!(back.last_check_slot, 9000);
        assert_eq!(back.pending, Some(Action::TopUp { amount: 7 }));
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
            last_check_slot: 0,
            pending: None,
        };
        g.write_into(&mut buf).unwrap();
        let back = GuardState::from_bytes(&buf).unwrap();
        assert_eq!(back.policy.take_profit, None);
        assert_eq!(back.pending, None);
    }

    #[test]
    fn route_config_roundtrip() {
        let mut buf = [0u8; ROUTE_CONFIG_LEN];
        let c = RouteConfig { authority: [3u8; 32], paused: true, _padding: [0u8; 31] };
        c.write_into(&mut buf).unwrap();
        let back = RouteConfig::from_bytes(&buf).unwrap();
        assert_eq!(back.authority, [3u8; 32]);
        assert!(back.paused);
    }
}
