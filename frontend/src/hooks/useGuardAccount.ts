'use client';

import { useEffect, useRef, useState } from 'react';
import { Connection, PublicKey } from '@solana/web3.js';
import {
  GUARD_DATA_LEN,
  VENUE_DRIFT,
  VENUE_JUPITER,
  VENUE_NONE,
  decodeGuardState,
  type GuardState,
} from '@/lib/guard-layout';
import { decodeRouteConfig, guardPda, routeConfigPda } from '@/lib/guard-program';
import { computeHealth, type Health } from '@/lib/guard-health';
import { useWallet } from './useWallet';

const POLL_INTERVAL_MS = 5_000;
const DEFAULT_RPC = 'https://api.devnet.solana.com';

export interface GuardSnapshot {
  address: string;
  state: GuardState;
  health: Health;
  venueLabel: string;
  /** True when the guard can only build for the owner to co-sign (§8.4). */
  isCoSigned: boolean;
  /** A guard-built instruction is waiting on the owner's signature. */
  awaitingConfirmation: boolean;
  /** True when the connected wallet is this guard's venue owner. */
  isOwner: boolean;
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

function readConfig(): { programId: PublicKey | null; rpc: string; error: string | null } {
  const rpc = process.env.NEXT_PUBLIC_SOLANA_RPC || DEFAULT_RPC;
  const raw = process.env.NEXT_PUBLIC_GUARD_PROGRAM_ID;
  if (!raw) {
    return { programId: null, rpc, error: 'NEXT_PUBLIC_GUARD_PROGRAM_ID is not set' };
  }
  try {
    return { programId: new PublicKey(raw), rpc, error: null };
  } catch {
    return { programId: null, rpc, error: `NEXT_PUBLIC_GUARD_PROGRAM_ID is not a valid pubkey` };
  }
}

export function useGuardAccount() {
  const { programId, rpc, error: configError } = readConfig();
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

  useEffect(() => {
    if (!programIdKey) return;

    const connection = new Connection(rpc, 'confirmed');
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
        const [found, configInfo] = await Promise.all([
          readGuard(),
          connection.getAccountInfo(routeConfig),
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
        const venueOwner = new PublicKey(state.venueOwner).toBase58();

        setSnapshot({
          address: found.address.toBase58(),
          state,
          health,
          venueLabel: venueLabel(state.venue, state.authorityReq),
          isCoSigned,
          awaitingConfirmation: isCoSigned && state.pendingIxNonce !== null,
          isOwner: ownerKey === venueOwner,
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
  }, [programIdKey, rpc, ownerKey, nudge]);

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
