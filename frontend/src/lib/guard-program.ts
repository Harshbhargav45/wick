/**
 * Client-side instruction builders for the Wick guard program.
 *
 * Discriminators mirror `WickInstruction` in `program/src/instruction.rs`; the
 * payload layouts and account orders mirror the handlers in
 * `program/src/processor.rs`. Both sides have to move together.
 */

import { Buffer } from 'buffer';
import { PublicKey, TransactionInstruction } from '@solana/web3.js';
import { ACCOUNT_VERSION } from './guard-layout';

export const IX_DEPOSIT_MARGIN = 1;
export const IX_UPDATE_POSITION = 8;
export const IX_CONFIRM_YES = 9;

const GUARD_SEED = new TextEncoder().encode('guard');
const ROUTE_CONFIG_SEED = new TextEncoder().encode('route_config');

/** `RouteConfig`: [0]=version, [1..33]=authority, [33]=paused. */
export const ROUTE_CONFIG_LEN = 34;

export interface RouteConfig {
  authority: string;
  paused: boolean;
}

export function decodeRouteConfig(data: Uint8Array): RouteConfig {
  if (data.length !== ROUTE_CONFIG_LEN) {
    throw new Error(`route config: expected ${ROUTE_CONFIG_LEN} bytes, got ${data.length}`);
  }
  if (data[0] !== ACCOUNT_VERSION) {
    throw new Error(`route config: unsupported version ${data[0]}`);
  }
  return {
    authority: new PublicKey(data.subarray(1, 33)).toBase58(),
    paused: data[33] === 1,
  };
}

/** `b"guard" || owner` — one guard per owner, per `init_guard`. */
export function guardPda(programId: PublicKey, owner: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([GUARD_SEED, owner.toBytes()], programId);
}

/** `b"route_config"` — the singleton kill-switch account. */
export function routeConfigPda(programId: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([ROUTE_CONFIG_SEED], programId);
}

function writeU128LE(out: Uint8Array, off: number, value: bigint): void {
  if (value < 0n) throw new Error('writeU128LE: negative value');
  let v = value;
  for (let i = 0; i < 16; i++) {
    out[off + i] = Number(v & 0xffn);
    v >>= 8n;
  }
}

function writeI128LE(out: Uint8Array, off: number, value: bigint): void {
  writeU128LE(out, off, value < 0n ? (1n << 128n) + value : value);
}

/**
 * Every state-mutating handler takes the guard first, the owner as signer, and
 * the route config last so it can check the kill-switch before doing anything.
 */
function ownerSignedKeys(guard: PublicKey, owner: PublicKey, routeConfig: PublicKey) {
  return [
    { pubkey: guard, isSigner: false, isWritable: true },
    { pubkey: owner, isSigner: true, isWritable: false },
    { pubkey: routeConfig, isSigner: false, isWritable: false },
  ];
}

/**
 * §8.4 — commit the pending nonce after the owner has landed the guard-built
 * venue instruction. Carries no payload: the pending instruction is on the
 * guard account.
 */
export function confirmYesIx(
  programId: PublicKey,
  guard: PublicKey,
  owner: PublicKey,
  routeConfig: PublicKey,
): TransactionInstruction {
  return new TransactionInstruction({
    programId,
    keys: ownerSignedKeys(guard, owner, routeConfig),
    data: Buffer.from([IX_CONFIRM_YES]),
  });
}

/** Add collateral to the guard's margin wallet. Owner only. */
export function depositMarginIx(
  programId: PublicKey,
  guard: PublicKey,
  owner: PublicKey,
  routeConfig: PublicKey,
  amount: bigint,
): TransactionInstruction {
  const data = new Uint8Array(17);
  data[0] = IX_DEPOSIT_MARGIN;
  writeU128LE(data, 1, amount);
  return new TransactionInstruction({
    programId,
    keys: ownerSignedKeys(guard, owner, routeConfig),
    data: Buffer.from(data),
  });
}

/**
 * Enrollment: record the watched position so the guard has real state to run
 * health against. `size` is signed — negative is short.
 */
export function updatePositionIx(
  programId: PublicKey,
  guard: PublicKey,
  owner: PublicKey,
  routeConfig: PublicKey,
  position: { collateral: bigint; size: bigint; entry: bigint },
): TransactionInstruction {
  const data = new Uint8Array(49);
  data[0] = IX_UPDATE_POSITION;
  writeU128LE(data, 1, position.collateral);
  writeI128LE(data, 17, position.size);
  writeU128LE(data, 33, position.entry);
  return new TransactionInstruction({
    programId,
    keys: ownerSignedKeys(guard, owner, routeConfig),
    data: Buffer.from(data),
  });
}
