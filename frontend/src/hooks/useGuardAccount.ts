import { useEffect, useState } from 'react';
import { Connection, PublicKey } from '@solana/web3.js';

export interface GuardUIState {
  healthFactor: number;
  isPending: boolean;
  isDegraded: boolean;
  venue: string;
  action: string | null;
  lastCheckSlot: number;
}

const GUARD_DATA_LEN = 342;
const SCALE = 1_000_000;

function parseProgramId(): { programId: PublicKey | null; error: string | null } {
  const programIdStr = process.env.NEXT_PUBLIC_GUARD_PROGRAM_ID;
  if (!programIdStr) return { programId: null, error: 'NEXT_PUBLIC_GUARD_PROGRAM_ID not set' };
  try {
    return { programId: new PublicKey(programIdStr), error: null };
  } catch {
    return { programId: null, error: 'Invalid program ID' };
  }
}

export function useGuardAccount() {
  const { programId, error: configError } = parseProgramId();
  const [state, setState] = useState<GuardUIState | null>(null);
  const [error, setError] = useState<string | null>(configError);

  useEffect(() => {
    if (!programId) return;

    // Connect to devnet
    const connection = new Connection('https://api.devnet.solana.com', 'confirmed');

    const fetchState = async () => {
      try {
        const accounts = await connection.getProgramAccounts(programId, {
          filters: [{ dataSize: GUARD_DATA_LEN }],
        });

        if (accounts.length === 0) {
          setError('No guard accounts found');
          return;
        }

        const data = accounts[0].account.data;

        // Parse the byte layout based on account.rs
        // G_VENUE_OFF = 1
        const venueByte = data[1];
        const venue = venueByte === 0 ? 'Drift · delegated' : venueByte === 1 ? 'Jupiter' : 'Unknown';

        // COLLAT = 179
        const collat = data.readBigUInt64LE(179); // simplified since it's u128 but JS can handle u64 easily if numbers are small
        // SIZE = 195 (i128) - we will approximate for UI
        const sizeData = data.readBigInt64LE(195);
        const entryData = data.readBigUInt64LE(211);
        const priceData = data.readBigUInt64LE(227);

        const size = Number(sizeData) / SCALE;
        const entry = Number(entryData) / SCALE;
        const price = Number(priceData) / SCALE;

        // Math matches state.rs
        const pnl = size * (price - entry);
        const equity = Number(collat) / SCALE + pnl;

        // Assuming 500 bps (5%) maintenance for demo if we don't parse policy completely
        const maintenanceBps = 500;
        const required = Math.abs(size) * price * (maintenanceBps / 10000);

        const healthFactor = required === 0 ? 99 : equity / required;

        // Pending Tag = 259
        const pendingTag = data[259];
        const isPending = pendingTag !== 0;
        let action: string | null = null;
        if (pendingTag === 1) action = 'TopUp';
        if (pendingTag === 2) action = 'PartialClose';
        if (pendingTag === 3) action = 'TakeProfit';
        if (pendingTag === 4) action = 'Escalate';

        // Degraded = 276
        const isDegraded = data[276] === 1;

        // Slot = 251
        const slot = Number(data.readBigUInt64LE(251));

        setState({
          healthFactor,
          isPending,
          isDegraded,
          venue,
          action,
          lastCheckSlot: slot,
        });
        setError(null);
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      }
    };

    fetchState();
    const interval = setInterval(fetchState, 5000);

    return () => clearInterval(interval);
  }, [programId]);

  return { state, error };
}