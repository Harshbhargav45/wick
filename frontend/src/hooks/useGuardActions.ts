'use client';

import { useState, useTransition } from 'react';
import { PublicKey, Transaction, type TransactionInstruction } from '@solana/web3.js';
import {
  closeGuardIx,
  confirmYesIx,
  depositMarginIx,
  explainProgramError,
  fundMarginWalletIx,
  guardPda,
  initMarginWalletIx,
  marginWalletAddressForBump,
  marginWalletPda,
  routeConfigPda,
  setPausedIx,
  updatePositionIx,
  withdrawMarginIx,
  withdrawMarginWalletIx,
} from '@/lib/guard-program';
import { createFailoverConnection, rpcEndpoints } from '@/lib/rpc';
import { canPartialSign, partialSignWithWallet, sendWithWallet } from '@/lib/wallet';

/**
 * Which operation a `TxState` belongs to, so several buttons can share one
 * state slot without one of them showing another's result.
 */
export type OpKind =
  | 'confirm'
  | 'deposit'
  | 'withdraw'
  | 'updatePosition'
  | 'initReserve'
  | 'fundReserve'
  | 'withdrawReserve'
  | 'closeGuard'
  | 'setPaused';

export type TxState =
  | { kind: 'idle' }
  | { kind: 'sending'; op: OpKind }
  | { kind: 'sent'; op: OpKind; signature: string }
  /**
   * A 2-of-2 that has the owner's signature and needs the co-authority's. Not
   * an error and not a success — the transaction is real and unsent, and saying
   * "sent" here would claim value moved when it did not.
   */
  | { kind: 'awaitingCoSign'; op: OpKind; base64: string }
  | { kind: 'error'; op: OpKind; message: string };

/**
 * Sends owner-signed guard instructions through the injected wallet.
 *
 * The guard never signs these — under §8.4 CoSigned the program only *builds*
 * the venue instruction and holds it. `confirm` is the owner recording that
 * they landed it, which is what advances the nonce.
 *
 * Two operations (`WithdrawMargin`, `WithdrawMarginWallet`) are 2-of-2 and
 * cannot complete in a browser holding one key. Those produce a partially
 * signed transaction for the co-authority instead of being sent and failing.
 */
export function useGuardActions(args: {
  programId: string | null;
  guardAddress: string | null;
  owner: PublicKey | null;
  /** The guard's co-authority, from the account. Needed for the 2-of-2 paths. */
  coAuthority: string | null;
  /** The reserve bump the guard recorded, or `null` when no reserve exists. */
  marginWalletBump: number | null;
  onDone?: () => void;
}) {
  const { programId, guardAddress, owner, coAuthority, marginWalletBump, onDone } = args;
  const [tx, setTx] = useState<TxState>({ kind: 'idle' });
  // `isPending` covers the window between the click and the wallet prompt
  // resolving, which `tx.kind === 'sending'` alone misses: the state update that
  // sets it is itself inside the transition.
  const [isPending, startTransition] = useTransition();

  const canSend = Boolean(programId && guardAddress && owner);

  interface BuildCtx {
    program: PublicKey;
    guard: PublicKey;
    owner: PublicKey;
    routeConfig: PublicKey;
  }

  /**
   * Build, sign, send, confirm. `coSign: true` stops after the owner's
   * signature and returns the bytes for the second signer.
   */
  const run = async (
    op: OpKind,
    build: (ctx: BuildCtx) => TransactionInstruction,
    coSign = false,
  ) => {
    if (!programId || !guardAddress || !owner) return;
    setTx({ kind: 'sending', op });
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

      if (coSign) {
        const signed = await partialSignWithWallet(transaction);
        const base64 = signed
          .serialize({ requireAllSignatures: false, verifySignatures: false })
          .toString('base64');
        // Post-`await`, so React does not fold this into the transition on its
        // own. Without the wrapper the panel would flip out of `sending` on a
        // separate frame from the result landing.
        startTransition(() => setTx({ kind: 'awaitingCoSign', op, base64 }));
        return;
      }

      const signature = await sendWithWallet(transaction);
      await connection.confirmTransaction(
        { signature, blockhash, lastValidBlockHeight },
        'confirmed',
      );
      startTransition(() => setTx({ kind: 'sent', op, signature }));
      onDone?.();
    } catch (err) {
      startTransition(() => setTx({ kind: 'error', op, message: explainProgramError(err) }));
    }
  };

  const dispatch = (op: OpKind, build: (ctx: BuildCtx) => TransactionInstruction, coSign = false) =>
    startTransition(() => {
      void run(op, build, coSign);
    });

  return {
    tx,
    canSend,
    isPending,
    /** True when the 2-of-2 paths can produce the owner's half in this browser. */
    canCoSign: canPartialSign(),

    confirm: () =>
      dispatch('confirm', ({ program, guard, owner: o, routeConfig }) =>
        confirmYesIx(program, guard, o, routeConfig),
      ),

    /** Credit the guard's recorded collateral. USD, 6dp. */
    deposit: (amount: bigint) =>
      dispatch('deposit', ({ program, guard, owner: o, routeConfig }) =>
        depositMarginIx(program, guard, o, routeConfig, amount),
      ),

    /** Debit recorded collateral — 2-of-2, so this hands off rather than sends. */
    withdraw: (amount: bigint) =>
      dispatch(
        'withdraw',
        ({ program, guard, owner: o, routeConfig }) => {
          if (!coAuthority) throw new Error('The guard has no co-authority recorded.');
          return withdrawMarginIx(
            program,
            guard,
            o,
            new PublicKey(coAuthority),
            routeConfig,
            amount,
          );
        },
        true,
      ),

    /** Re-enroll the watched position. Also the fix for a diverged guard. */
    updatePosition: (position: { collateral: bigint; size: bigint; entry: bigint }) =>
      dispatch('updatePosition', ({ program, guard, owner: o, routeConfig }) =>
        updatePositionIx(program, guard, o, routeConfig, position),
      ),

    /** Create the lamport reserve that backs autonomous top-ups (§8.5). */
    initReserve: () =>
      dispatch('initReserve', ({ program, guard, owner: o, routeConfig }) => {
        const [wallet, bump] = marginWalletPda(program, o);
        return initMarginWalletIx(program, wallet, guard, o, routeConfig, bump);
      }),

    /** Move real lamports into the reserve. */
    fundReserve: (lamports: bigint) =>
      dispatch('fundReserve', ({ program, guard, owner: o, routeConfig }) => {
        if (marginWalletBump === null) {
          throw new Error('No reserve exists for this guard yet — create one first.');
        }
        // Derived from the bump the guard recorded, because that is the bump
        // `verify_margin_wallet` re-derives with. Deriving canonically would
        // address a different account for a reserve created under any other.
        const wallet = marginWalletAddressForBump(program, o, marginWalletBump);
        if (!wallet) throw new Error(`The recorded reserve bump ${marginWalletBump} is not valid.`);
        return fundMarginWalletIx(program, wallet, guard, o, routeConfig, lamports);
      }),

    /** Take lamports back out — 2-of-2. */
    withdrawReserve: (lamports: bigint) =>
      dispatch(
        'withdrawReserve',
        ({ program, guard, owner: o, routeConfig }) => {
          if (!coAuthority) throw new Error('The guard has no co-authority recorded.');
          if (marginWalletBump === null) {
            throw new Error('No reserve exists for this guard yet.');
          }
          const wallet = marginWalletAddressForBump(program, o, marginWalletBump);
          if (!wallet) {
            throw new Error(`The recorded reserve bump ${marginWalletBump} is not valid.`);
          }
          return withdrawMarginWalletIx(
            program,
            wallet,
            guard,
            o,
            new PublicKey(coAuthority),
            routeConfig,
            lamports,
          );
        },
        true,
      ),

    /** Close the guard and reclaim its rent. Irreversible. */
    closeGuard: () =>
      dispatch('closeGuard', ({ program, guard, owner: o }) => {
        const [, bump] = guardPda(program, o);
        return closeGuardIx(program, guard, o, bump);
      }),

    /**
     * The program-wide kill switch. Signed by the route authority, which is
     * usually not the guard owner — the program will reject any other signer.
     */
    setPaused: (paused: boolean) =>
      dispatch('setPaused', ({ program, owner: o, routeConfig }) =>
        setPausedIx(program, routeConfig, o, paused),
      ),

    reset: () => setTx({ kind: 'idle' }),
  };
}
