/**
 * MagicBlock Ephemeral Rollup wiring: the well-known program ids, the three PDAs
 * `delegate_account` touches, and the instruction builders for wick's Delegate /
 * Commit / CommitAndUndelegate (§8.6).
 *
 * Kept separate from delegate.mjs so importing any of this does not run that
 * file's CLI. Mirrors program/src/delegation.rs and the account order in
 * ephemeral-rollups-pinocchio 0.16.2 (`cpi_delegate`).
 */
import { PublicKey } from "@solana/web3.js";
import { Buffer as IsomorphicBuffer } from "node:buffer";
import { config } from "./config.mjs";

export const IX_DELEGATE = 4;
export const IX_COMMIT_AND_UNDELEGATE = 5;
export const IX_COMMIT = 6;

export const DELEGATION_PROGRAM_ID = new PublicKey(
  "DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh"
);
export const MAGIC_PROGRAM_ID = new PublicKey(
  "Magic11111111111111111111111111111111111111"
);
export const MAGIC_CONTEXT_ID = new PublicKey(
  "MagicContext1111111111111111111111111111111"
);
export const SYSTEM_PROGRAM_ID = new PublicKey(
  "11111111111111111111111111111111"
);

/**
 * The buffer derives under the *owner* program (wick), not the delegation
 * program: the SDK creates it owned by wick so it can memcpy the guard into it
 * before the guard is assigned away. Record and metadata belong to the
 * delegation program.
 */
export function delegationPdas(guard) {
  const [buffer] = PublicKey.findProgramAddressSync(
    [IsomorphicBuffer.from("buffer"), guard.toBuffer()],
    config.wickProgramId
  );
  const [record] = PublicKey.findProgramAddressSync(
    [IsomorphicBuffer.from("delegation"), guard.toBuffer()],
    DELEGATION_PROGRAM_ID
  );
  const [metadata] = PublicKey.findProgramAddressSync(
    [IsomorphicBuffer.from("delegation-metadata"), guard.toBuffer()],
    DELEGATION_PROGRAM_ID
  );
  return { buffer, record, metadata };
}

export function delegateIx({ payer, guard, bump, validator }) {
  const { buffer, record, metadata } = delegationPdas(guard);
  const data = IsomorphicBuffer.alloc(2);
  data[0] = IX_DELEGATE;
  data[1] = bump;
  const keys = [
    { pubkey: payer, isSigner: true, isWritable: true },
    { pubkey: guard, isSigner: false, isWritable: true },
    { pubkey: config.wickProgramId, isSigner: false, isWritable: false },
    { pubkey: buffer, isSigner: false, isWritable: true },
    { pubkey: record, isSigner: false, isWritable: true },
    { pubkey: metadata, isSigner: false, isWritable: true },
    { pubkey: DELEGATION_PROGRAM_ID, isSigner: false, isWritable: false },
    { pubkey: SYSTEM_PROGRAM_ID, isSigner: false, isWritable: false },
  ];
  // `rest.first()` in process_delegate — an optional 9th account pins the ER
  // validator. Omitted means the delegation program picks one.
  if (validator) {
    keys.push({ pubkey: validator, isSigner: false, isWritable: false });
  }
  return { programId: config.wickProgramId, keys, data };
}

/** Commit (6) and CommitAndUndelegate (5) share one account layout. */
export function magicIx({ discriminator, payer, guard }) {
  return {
    programId: config.wickProgramId,
    keys: [
      { pubkey: payer, isSigner: true, isWritable: true },
      { pubkey: guard, isSigner: false, isWritable: true },
      { pubkey: MAGIC_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: MAGIC_CONTEXT_ID, isSigner: false, isWritable: true },
    ],
    data: IsomorphicBuffer.from([discriminator]),
  };
}

/**
 * `owner == wick` means opposite things on the two layers: on the base layer the
 * guard is still local, while on the ER it means the ER has hydrated the guard
 * and will accept writes to it. Both readings are just "owned by wick", so the
 * layer has to be passed in.
 */
export function describeOwner(owner, layer = "base") {
  if (owner.equals(config.wickProgramId)) {
    return layer === "er"
      ? "wick (hydrated, writable on ER)"
      : "wick (base layer, undelegated)";
  }
  if (owner.equals(DELEGATION_PROGRAM_ID)) return "DELEGATED to MagicBlock ER";
  return owner.toBase58();
}
