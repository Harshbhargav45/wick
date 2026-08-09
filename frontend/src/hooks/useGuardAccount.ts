'use client';

import { useEffect, useRef, useState } from 'react';
import { Connection, PublicKey } from '@solana/web3.js';
import {
  GUARD_DATA_LEN,
  VENUE_DRIFT,
  VENUE_JUPITER,
  decodeGuardState,
  type GuardState,
} from '@/lib/guard-layout';
import { computeHealth, type Health } from '@/lib/guard-health';

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
  fetchedAt: number;
}

export type GuardStatus = 'config' | 'loading' | 'empty' | 'error' | 'ready';

function venueLabel(venue: number, authority: string): string {
  if (venue === VENUE_DRIFT) {
    return authority === 'Autonomous' ? 'Drift · delegated' : 'Drift · co-signed';
  }
  if (venue === VENUE_JUPITER) return 'Jupiter · co-signed';
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

  const [snapshot, setSnapshot] = useState<GuardSnapshot | null>(null);
  const [status, setStatus] = useState<GuardStatus>(configError ? 'config' : 'loading');
  const [error, setError] = useState<string | null>(configError);
  const [nudge, setNudge] = useState(0);

  const programIdKey = programId?.toBase58();
  const inFlight = useRef(false);

  useEffect(() => {
    if (!programIdKey) return;

    const connection = new Connection(rpc, 'confirmed');
    const key = new PublicKey(programIdKey);
    let cancelled = false;

    const fetchState = async () => {
      if (inFlight.current) return;
      inFlight.current = true;
      try {
        const accounts = await connection.getProgramAccounts(key, {
          filters: [{ dataSize: GUARD_DATA_LEN }],
        });
        if (cancelled) return;

        if (accounts.length === 0) {
          setSnapshot(null);
          setStatus('empty');
          setError(null);
          return;
        }

        const account = accounts[0]!;
        const state = decodeGuardState(new Uint8Array(account.account.data));
        const health = computeHealth(state);
        const isCoSigned = state.authorityReq === 'CoSigned';

        setSnapshot({
          address: account.pubkey.toBase58(),
          state,
          health,
          venueLabel: venueLabel(state.venue, state.authorityReq),
          isCoSigned,
          awaitingConfirmation: isCoSigned && state.pendingIxNonce !== null,
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
  }, [programIdKey, rpc, nudge]);

  const refresh = () => setNudge((n) => n + 1);

  return { snapshot, status, error, refresh, rpc, programId: programIdKey ?? null };
}
