'use client';

import { useEffect, useRef, useState } from 'react';
import { PublicKey } from '@solana/web3.js';
import {
  GUARD_DATA_LEN,
  VENUE_DRIFT,
  VENUE_JUPITER,
  VENUE_NONE,
  decodeGuardState,
  decodeWalletState,
  type GuardState,
  type WalletState,
} from '@/lib/guard-layout';
import {
  decodeRouteConfig,
  guardPda,
  marginWalletAddressForBump,
  marginWalletPda,
  routeConfigPda,
} from '@/lib/guard-program';
import { computeHealth, dailyBudget, type DailyBudget, type Health } from '@/lib/guard-health';
import { createFailoverConnection, redactRpc, rpcEndpoints } from '@/lib/rpc';
import { useWallet } from './useWallet';

const POLL_INTERVAL_MS = 5_000;

/** `SysvarC1ock` — read for `unix_timestamp`, which is the clock the program sees. */
const CLOCK_SYSVAR = new PublicKey('SysvarC1ock11111111111111111111111111111111');
/** `Clock`: slot(8) epoch_start_timestamp(8) epoch(8) leader_schedule_epoch(8) unix_timestamp(8). */
const CLOCK_UNIX_TS_OFF = 32;

function readClockUnixTs(data: Uint8Array): bigint {
  let acc = 0n;
  for (let i = 7; i >= 0; i--) acc = (acc << 8n) | BigInt(data[CLOCK_UNIX_TS_OFF + i]!);
  return acc >= 1n << 63n ? acc - (1n << 64n) : acc;
}

/**
 * The guard's margin reserve (§8.5), as the console needs to reason about it.
 *
 * `exists: false` is a real operational state, not an absence of data: the
 * program gates an autonomous `TopUp` on `margin_wallet_bump != 0`, so a guard
 * whose policy allows top-ups but has no reserve will escalate instead of
 * acting. That is worth saying out loud on the dashboard.
 */
export interface ReserveState {
  address: string;
  exists: boolean;
  /** Lamports the reserve claims to hold on the owner's behalf. */
  balance: bigint;
  /** Total lamports on the account, including the rent that is not withdrawable. */
  lamports: bigint;
  state: WalletState | null;
}

export interface GuardSnapshot {
  address: string;
  state: GuardState;
  health: Health;
  /** Daily action budget, with the epoch rollover already applied (§8.2). */
  budget: DailyBudget;
  venueLabel: string;
  /** True when the guard can only build for the owner to co-sign (§8.4). */
  isCoSigned: boolean;
  /** A guard-built instruction is waiting on the owner's signature. */
  awaitingConfirmation: boolean;
  /** True when the connected wallet is this guard's venue owner. */
  isOwner: boolean;
  /** The 2-of-2 lamport reserve behind autonomous top-ups. */
  reserve: ReserveState;
  /** The chain's own `unix_timestamp` at this read — what staleness is measured against. */
  chainTs: bigint;
  fetchedAt: number;
}

export type GuardStatus = 'config' | 'loading' | 'empty' | 'error' | 'ready';

/**
 * The kill-switch account. `exists: false` is not the same as "running":
 * `check_not_paused` rejects when the account is missing, so every
 * state-mutating instruction fails until it is initialized.
 */
export interface RouteConfigState {
  exists: boolean;
  paused: boolean;
}

function venueLabel(venue: number, authority: string): string {
  if (venue === VENUE_DRIFT) {
    return authority === 'Autonomous' ? 'Drift · delegated' : 'Drift · co-signed';
  }
  if (venue === VENUE_JUPITER) return 'Jupiter · co-signed';
  if (venue === VENUE_NONE) return 'No venue · watch only';
  return `Venue ${venue}`;
}

function readConfig(): {
  programId: PublicKey | null;
  endpoints: string[];
  rpc: string;
  error: string | null;
} {
  const endpoints = rpcEndpoints();
  // Displayed, so it is the redacted head rather than the keyed URL itself.
  const rpc = redactRpc(endpoints[0]);
  const raw = process.env.NEXT_PUBLIC_GUARD_PROGRAM_ID;
  if (!raw) {
    return {
      programId: null,
      endpoints,
      rpc,
      error: 'NEXT_PUBLIC_GUARD_PROGRAM_ID is not set',
    };
  }
  try {
    return { programId: new PublicKey(raw), endpoints, rpc, error: null };
  } catch {
    return {
      programId: null,
      endpoints,
      rpc,
      error: `NEXT_PUBLIC_GUARD_PROGRAM_ID is not a valid pubkey`,
    };
  }
}

export function useGuardAccount() {
  const { programId, endpoints, rpc, error: configError } = readConfig();
  const { publicKey } = useWallet();

  const [snapshot, setSnapshot] = useState<GuardSnapshot | null>(null);
  const [status, setStatus] = useState<GuardStatus>(configError ? 'config' : 'loading');
  const [error, setError] = useState<string | null>(configError);
  const [routeConfig, setRouteConfig] = useState<RouteConfigState>({
    exists: false,
    paused: false,
  });
  const [nudge, setNudge] = useState(0);

  const programIdKey = programId?.toBase58();
  const ownerKey = publicKey?.toBase58() ?? null;
  const inFlight = useRef(false);
  // `endpoints` is a fresh array every render, so the effect keys off its
  // contents rather than its identity — otherwise it tears down and rebuilds
  // the poll interval on every render.
  const endpointKey = endpoints.join(',');

  useEffect(() => {
    if (!programIdKey) return;

    const connection = createFailoverConnection(endpointKey.split(','));
    const program = new PublicKey(programIdKey);
    const owner = ownerKey ? new PublicKey(ownerKey) : null;
    const [routeConfig] = routeConfigPda(program);
    let cancelled = false;

    /**
     * A connected wallet addresses its own guard directly by PDA. Without one
     * the console is read-only, so it falls back to scanning program accounts
     * for any guard to display.
     */
    const readGuard = async (): Promise<{ address: PublicKey; data: Uint8Array } | null> => {
      if (owner) {
        const [pda] = guardPda(program, owner);
        const info = await connection.getAccountInfo(pda);
        return info ? { address: pda, data: new Uint8Array(info.data) } : null;
      }
      const accounts = await connection.getProgramAccounts(program, {
        filters: [{ dataSize: GUARD_DATA_LEN }],
      });
      const first = accounts[0];
      return first ? { address: first.pubkey, data: new Uint8Array(first.account.data) } : null;
    };

    const fetchState = async () => {
      if (inFlight.current) return;
      inFlight.current = true;
      try {
        const [found, configInfo, clockInfo] = await Promise.all([
          readGuard(),
          connection.getAccountInfo(routeConfig),
          connection.getAccountInfo(CLOCK_SYSVAR),
        ]);
        if (cancelled) return;

        setRouteConfig(
          configInfo
            ? { exists: true, paused: decodeRouteConfig(new Uint8Array(configInfo.data)).paused }
            : { exists: false, paused: false },
        );

        if (!found) {
          setSnapshot(null);
          setStatus('empty');
          setError(null);
          return;
        }

        const state = decodeGuardState(found.data);
        const health = computeHealth(state);
        const isCoSigned = state.authorityReq === 'CoSigned';
        const venueOwnerKey = new PublicKey(state.venueOwner);
        const venueOwner = venueOwnerKey.toBase58();

        // The reserve is derived from the guard's own `venue_owner` rather than
        // the connected wallet, so a read-only viewer sees the same reserve the
        // program would spend from.
        const reserveAddress =
          state.marginWalletBump === null
            ? marginWalletPda(program, venueOwnerKey)[0]
            : (marginWalletAddressForBump(program, venueOwnerKey, state.marginWalletBump) ??
              marginWalletPda(program, venueOwnerKey)[0]);
        const reserveInfo = state.marginWalletBump === null
          ? null
          : await connection.getAccountInfo(reserveAddress);
        if (cancelled) return;

        const reserveState = reserveInfo
          ? decodeWalletState(new Uint8Array(reserveInfo.data))
          : null;
        const reserve: ReserveState = {
          address: reserveAddress.toBase58(),
          exists: reserveState !== null,
          balance: reserveState?.balance ?? 0n,
          lamports: BigInt(reserveInfo?.lamports ?? 0),
          state: reserveState,
        };

        // The program's own clock, not the browser's — a machine with a skewed
        // system time would otherwise roll the daily epoch at the wrong moment
        // and report a budget the program does not agree with.
        const chainTs = clockInfo
          ? readClockUnixTs(new Uint8Array(clockInfo.data))
          : BigInt(Math.floor(Date.now() / 1000));

        setSnapshot({
          address: found.address.toBase58(),
          state,
          health,
          budget: dailyBudget(state, chainTs),
          venueLabel: venueLabel(state.venue, state.authorityReq),
          isCoSigned,
          awaitingConfirmation: isCoSigned && state.pendingIxNonce !== null,
          isOwner: ownerKey === venueOwner,
          reserve,
          chainTs,
          fetchedAt: Date.now(),
        });
        setStatus('ready');
        setError(null);
      } catch (err) {
        if (cancelled) return;
        setStatus((prev) => (prev === 'ready' ? 'ready' : 'error'));
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        inFlight.current = false;
      }
    };

    void fetchState();
    const interval = setInterval(fetchState, POLL_INTERVAL_MS);

    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [programIdKey, endpointKey, ownerKey, nudge]);

  const refresh = () => setNudge((n) => n + 1);

  return {
    snapshot,
    status,
    error,
    routeConfig,
    /** Instructions are blocked when paused *or* when the config is missing. */
    writesBlocked: !routeConfig.exists || routeConfig.paused,
    refresh,
    rpc,
    programId: programIdKey ?? null,
  };
}

/**
 * Why a write cannot be attempted, or `null` when it can.
 *
 * Centralized so every button gives the same reason for the same condition.
 * These are the conditions the *program* will reject on, checked here only so
 * the owner reads a sentence instead of a simulation log — the program remains
 * the authority, and nothing here is a substitute for its checks.
 */
export function writeBlockReason(
  routeConfig: RouteConfigState,
  snapshot: GuardSnapshot | null,
  connected: boolean,
): string | null {
  if (!connected) return 'Connect a wallet to sign.';
  if (!routeConfig.exists) return 'RouteConfig is not initialized — every write rejects.';
  if (routeConfig.paused) return 'The program is paused by the route authority.';
  if (snapshot && !snapshot.isOwner) return 'The connected wallet is not this guard’s owner.';
  return null;
}
