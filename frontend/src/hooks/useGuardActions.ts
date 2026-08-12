'use client';

import { useState } from 'react';
import { PublicKey, Transaction, type TransactionInstruction } from '@solana/web3.js';
import { confirmYesIx, routeConfigPda } from '@/lib/guard-program';
import { createFailoverConnection, rpcEndpoints } from '@/lib/rpc';
import { sendWithWallet } from '@/lib/wallet';

export type TxState =
  | { kind: 'idle' }
  | { kind: 'sending' }
  | { kind: 'sent'; signature: string }
  | { kind: 'error'; message: string };

/**
 * Sends owner-signed guard instructions through the injected wallet.
 *
 * The guard never signs these — under §8.4 CoSigned the program only *builds*
 * the venue instruction and holds it. `confirm` is the owner recording that
 * they landed it, which is what advances the nonce.
 */
export function useGuardActions(args: {
  programId: string | null;
  guardAddress: string | null;
  owner: PublicKey | null;
  onDone?: () => void;
}) {
  const { programId, guardAddress, owner, onDone } = args;
  const [tx, setTx] = useState<TxState>({ kind: 'idle' });

  const canSend = Boolean(programId && guardAddress && owner);

  const send = async (build: (ctx: {
    program: PublicKey;
    guard: PublicKey;
    owner: PublicKey;
    routeConfig: PublicKey;
  }) => TransactionInstruction) => {
    if (!programId || !guardAddress || !owner) return;
    setTx({ kind: 'sending' });
    try {
      const program = new PublicKey(programId);
      const guard = new PublicKey(guardAddress);
      const [routeConfig] = routeConfigPda(program);

      const connection = createFailoverConnection(rpcEndpoints());
      const { blockhash, lastValidBlockHeight } = await connection.getLatestBlockhash('confirmed');

      const transaction = new Transaction({
        feePayer: owner,
        blockhash,
        lastValidBlockHeight,
      }).add(build({ program, guard, owner, routeConfig }));

      const signature = await sendWithWallet(transaction);
      await connection.confirmTransaction(
        { signature, blockhash, lastValidBlockHeight },
        'confirmed',
      );
      setTx({ kind: 'sent', signature });
      onDone?.();
    } catch (err) {
      setTx({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  };

  const confirm = () =>
    send(({ program, guard, owner: o, routeConfig }) => confirmYesIx(program, guard, o, routeConfig));

  const reset = () => setTx({ kind: 'idle' });

  return { tx, canSend, confirm, reset };
}
