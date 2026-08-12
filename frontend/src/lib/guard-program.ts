/**
 * Client-side instruction builders for the Wick guard program.
 *
 * Discriminators mirror the `WickInstruction` enum in
 * `program/src/instruction.rs`; the payload layouts and account orders mirror
 * the handler doc comments in `program/src/processor.rs`. Both sides have to
 * move together — an account in the wrong slot does not fail cleanly, it
 * addresses a different account than the handler expects.
 *
 * The discriminators are written out in full rather than only the ones used
 * here. The cranker's copy of this table once carried a partial map whose gaps
 * had been closed up, renumbering everything after them, and a wrong
 * discriminator builds a transaction that silently invokes the wrong handler.
 */

import { Buffer } from 'buffer';
import { PublicKey, TransactionInstruction } from '@solana/web3.js';
import { ACCOUNT_VERSION } from './guard-layout';

export const IX_INIT_GUARD = 0;
export const IX_DEPOSIT_MARGIN = 1;
export const IX_WITHDRAW_MARGIN = 2;
export const IX_SET_PAUSED = 3;
export const IX_DELEGATE = 4;
export const IX_COMMIT_AND_UNDELEGATE = 5;
export const IX_COMMIT = 6;
export const IX_ON_PRICE_TICK = 7;
export const IX_UPDATE_POSITION = 8;
export const IX_CONFIRM_YES = 9;
export const IX_INIT_ROUTE_CONFIG = 10;
export const IX_CLOSE_GUARD = 11;
export const IX_SET_ROUTE_AUTHORITY = 12;
export const IX_RECONCILE_VENUE = 13;
export const IX_INIT_MARGIN_WALLET = 14;
export const IX_FUND_MARGIN_WALLET = 15;
export const IX_WITHDRAW_MARGIN_WALLET = 16;

const GUARD_SEED = new TextEncoder().encode('guard');
const ROUTE_CONFIG_SEED = new TextEncoder().encode('route_config');
const MARGIN_WALLET_SEED = new TextEncoder().encode('margin');

const SYSTEM_PROGRAM = new PublicKey('11111111111111111111111111111111');
const RENT_SYSVAR = new PublicKey('SysvarRent111111111111111111111111111111111');

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

/**
 * `b"margin" || owner` — the guard's 2-of-2 lamport reserve (§8.5).
 *
 * Seeded on the *owner*, not the guard, so it is derivable before the guard has
 * been read. The program re-derives it from `venue_owner` on every margin
 * instruction and refuses anything else.
 */
export function marginWalletPda(programId: PublicKey, owner: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([MARGIN_WALLET_SEED, owner.toBytes()], programId);
}

/**
 * The reserve address for a bump the guard has already recorded.
 *
 * `verify_margin_wallet` re-derives with the *stored* bump rather than the
 * canonical one, so a reserve created under a non-canonical bump is valid to the
 * program. Deriving canonically here would then read a different, empty address
 * and report a funded reserve as missing. Returns `null` if the bump does not
 * yield a valid address.
 */
export function marginWalletAddressForBump(
  programId: PublicKey,
  owner: PublicKey,
  bump: number,
): PublicKey | null {
  try {
    return PublicKey.createProgramAddressSync(
      [MARGIN_WALLET_SEED, owner.toBytes(), Uint8Array.of(bump)],
      programId,
    );
  } catch {
    return null;
  }
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

/** `[disc, amount u128 LE]` — the shared payload of every `parse_amount` handler. */
function amountData(disc: number, amount: bigint): Buffer {
  const data = new Uint8Array(17);
  data[0] = disc;
  writeU128LE(data, 1, amount);
  return Buffer.from(data);
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

/** Credit the guard's recorded collateral. Owner only. */
export function depositMarginIx(
  programId: PublicKey,
  guard: PublicKey,
  owner: PublicKey,
  routeConfig: PublicKey,
  amount: bigint,
): TransactionInstruction {
  return new TransactionInstruction({
    programId,
    keys: ownerSignedKeys(guard, owner, routeConfig),
    data: amountData(IX_DEPOSIT_MARGIN, amount),
  });
}

/**
 * Debit the guard's recorded collateral — 2-of-2 (§8.5).
 *
 * Account layout: [0] guard (w), [1] owner (signer), [2] co_authority (signer),
 * [3] route_config.
 */
export function withdrawMarginIx(
  programId: PublicKey,
  guard: PublicKey,
  owner: PublicKey,
  coAuthority: PublicKey,
  routeConfig: PublicKey,
  amount: bigint,
): TransactionInstruction {
  return new TransactionInstruction({
    programId,
    keys: [
      { pubkey: guard, isSigner: false, isWritable: true },
      { pubkey: owner, isSigner: true, isWritable: false },
      { pubkey: coAuthority, isSigner: true, isWritable: false },
      { pubkey: routeConfig, isSigner: false, isWritable: false },
    ],
    data: amountData(IX_WITHDRAW_MARGIN, amount),
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

/**
 * The program-wide kill switch. Signed by the *route* authority, which is not
 * the guard owner — `set_paused` takes only the config and that authority.
 *
 * Account layout: [0] route_config (w), [1] authority (signer).
 */
export function setPausedIx(
  programId: PublicKey,
  routeConfig: PublicKey,
  authority: PublicKey,
  paused: boolean,
): TransactionInstruction {
  return new TransactionInstruction({
    programId,
    keys: [
      { pubkey: routeConfig, isSigner: false, isWritable: true },
      { pubkey: authority, isSigner: true, isWritable: false },
    ],
    data: Buffer.from([IX_SET_PAUSED, paused ? 1 : 0]),
  });
}

/**
 * Rotate the kill-switch authority. The incoming authority must sign — a
 * rotation to a mistyped address disables the kill switch permanently, and
 * RouteConfig is a singleton with no second address to fall back on.
 *
 * Account layout: [0] route_config (w), [1] current authority (signer),
 * [2] new authority (signer). No payload.
 */
export function setRouteAuthorityIx(
  programId: PublicKey,
  routeConfig: PublicKey,
  authority: PublicKey,
  newAuthority: PublicKey,
): TransactionInstruction {
  return new TransactionInstruction({
    programId,
    keys: [
      { pubkey: routeConfig, isSigner: false, isWritable: true },
      { pubkey: authority, isSigner: true, isWritable: false },
      { pubkey: newAuthority, isSigner: true, isWritable: false },
    ],
    data: Buffer.from([IX_SET_ROUTE_AUTHORITY]),
  });
}

/**
 * Close the guard and refund its rent to the owner.
 *
 * Deliberately takes no route config: `close_guard` is not gated on the kill
 * switch, because a pause exists to stop the guard from acting and trapping the
 * owner's rent for its duration is not part of that.
 *
 * Account layout: [0] guard (w), [1] owner (signer, w — receives the refund).
 * Data: [bump].
 */
export function closeGuardIx(
  programId: PublicKey,
  guard: PublicKey,
  owner: PublicKey,
  bump: number,
): TransactionInstruction {
  return new TransactionInstruction({
    programId,
    keys: [
      { pubkey: guard, isSigner: false, isWritable: true },
      { pubkey: owner, isSigner: true, isWritable: true },
    ],
    data: Buffer.from([IX_CLOSE_GUARD, bump]),
  });
}

/**
 * Create the guard's margin reserve and link its bump into the guard.
 *
 * Account layout: [0] wallet PDA (w, created), [1] guard (w), [2] owner
 * (signer), [3] payer (signer, w), [4] rent sysvar, [5] route_config,
 * [6] system program — required because the handler creates the account by CPI.
 * Data: [bump].
 */
export function initMarginWalletIx(
  programId: PublicKey,
  wallet: PublicKey,
  guard: PublicKey,
  owner: PublicKey,
  routeConfig: PublicKey,
  bump: number,
): TransactionInstruction {
  return new TransactionInstruction({
    programId,
    keys: [
      { pubkey: wallet, isSigner: false, isWritable: true },
      { pubkey: guard, isSigner: false, isWritable: true },
      { pubkey: owner, isSigner: true, isWritable: false },
      // Owner and payer are the same key in the console; the program takes them
      // as separate slots so a third party can fund the rent.
      { pubkey: owner, isSigner: true, isWritable: true },
      { pubkey: RENT_SYSVAR, isSigner: false, isWritable: false },
      { pubkey: routeConfig, isSigner: false, isWritable: false },
      { pubkey: SYSTEM_PROGRAM, isSigner: false, isWritable: false },
    ],
    data: Buffer.from([IX_INIT_MARGIN_WALLET, bump]),
  });
}

/**
 * Move real lamports from the owner into the reserve.
 *
 * Account layout: [0] wallet (w), [1] guard (ro), [2] owner (signer, w),
 * [3] rent sysvar, [4] route_config, [5] system program — the transfer is a
 * System CPI. Data: amount in **lamports** (u128 LE, must fit u64).
 */
export function fundMarginWalletIx(
  programId: PublicKey,
  wallet: PublicKey,
  guard: PublicKey,
  owner: PublicKey,
  routeConfig: PublicKey,
  lamports: bigint,
): TransactionInstruction {
  return new TransactionInstruction({
    programId,
    keys: [
      { pubkey: wallet, isSigner: false, isWritable: true },
      { pubkey: guard, isSigner: false, isWritable: false },
      { pubkey: owner, isSigner: true, isWritable: true },
      { pubkey: RENT_SYSVAR, isSigner: false, isWritable: false },
      { pubkey: routeConfig, isSigner: false, isWritable: false },
      { pubkey: SYSTEM_PROGRAM, isSigner: false, isWritable: false },
    ],
    data: amountData(IX_FUND_MARGIN_WALLET, lamports),
  });
}

/**
 * Withdraw lamports out of the reserve — 2-of-2 (§8.5).
 *
 * Carries **no** system program: the reserve is program-owned, so System will
 * not debit it and the handler moves lamports by direct mutation.
 *
 * Account layout: [0] wallet (w), [1] guard (ro), [2] owner (signer, w —
 * receives), [3] co_authority (signer), [4] rent sysvar, [5] route_config.
 * Data: amount in **lamports** (u128 LE, must fit u64).
 */
export function withdrawMarginWalletIx(
  programId: PublicKey,
  wallet: PublicKey,
  guard: PublicKey,
  owner: PublicKey,
  coAuthority: PublicKey,
  routeConfig: PublicKey,
  lamports: bigint,
): TransactionInstruction {
  return new TransactionInstruction({
    programId,
    keys: [
      { pubkey: wallet, isSigner: false, isWritable: true },
      { pubkey: guard, isSigner: false, isWritable: false },
      { pubkey: owner, isSigner: true, isWritable: true },
      { pubkey: coAuthority, isSigner: true, isWritable: false },
      { pubkey: RENT_SYSVAR, isSigner: false, isWritable: false },
      { pubkey: routeConfig, isSigner: false, isWritable: false },
    ],
    data: amountData(IX_WITHDRAW_MARGIN_WALLET, lamports),
  });
}

/**
 * `WickError` discriminants, mirroring `program/src/error.rs`.
 *
 * A `ProgramError::Custom(n)` reaches the browser as the string
 * `custom program error: 0x17` buried in a simulation log, which tells an owner
 * nothing about what to do next. These are the sentences to show instead.
 */
const ERROR_COPY: Record<number, string> = {
  0x0: 'The program did not recognize this instruction. The console and the deployed program are out of step.',
  0x1: 'That account is not owned by the guard program. A delegated guard has to be undelegated first.',
  0x2: 'PDA derivation did not match — the wrong guard or reserve address was passed.',
  0x3: 'Already initialized. This guard or reserve exists; re-initializing it would wipe funded state.',
  0x4: 'Not initialized. Run the cranker init step to create the RouteConfig first.',
  0x5: 'The co-authority did not sign. This action needs both keys.',
  0x6: 'The owner did not sign.',
  0x7: 'A required signer signed the wrong key — the connected wallet is not this guard’s owner.',
  0x8: 'The amount overflowed, or exceeds the balance available to debit.',
  0x9: 'No action within the configured caps can restore the buffer. This needs a manual decision.',
  0xa: 'The action exceeds the venue policy cap.',
  0xb: 'Replayed or stale nonce — this state was already committed.',
  0xc: 'Unauthorized: the signing key is not the route authority.',
  0xd: 'This venue adapter cannot execute the selected action.',
  0xe: 'The venue CPI failed inside the adapter.',
  0xf: 'Nothing is pending to confirm.',
  0x10: 'The program is paused. The route authority has to resume it before any write lands.',
  0x11: 'Tick nonce out of order.',
  0x12: 'The pending action is advisory for this venue — there is no guard-built instruction to confirm. Resolve it at the venue directly.',
  0x13: 'Stale reconcile nonce — a newer venue snapshot is already recorded.',
  0x14: 'The guard’s position disagrees with the venue. Autonomous execution stays blocked until you re-enroll the position with Update position.',
  0x15: 'That venue position account is not the one this guard watches.',
  0x16: 'The margin reserve cannot cover this — the recorded balance is lower than the amount, or the withdrawal would eat the rent that keeps the reserve alive.',
  0x17: 'That is not this guard’s margin reserve. Create one first, or check the derived address.',
  0x18: 'A defensive close cannot be built right now — most often the build request is past its TTL.',
};

/**
 * Turn a thrown send/simulate error into something an owner can act on.
 *
 * The custom code arrives in the message text rather than as a field on any
 * standard error shape, so this matches on the text and falls back to it
 * verbatim. Returning the raw message unchanged is the honest default — better
 * a cryptic true error than a confident wrong summary.
 */
export function explainProgramError(err: unknown): string {
  const message = err instanceof Error ? err.message : String(err);
  const match = /custom program error:\s*(0x[0-9a-fA-F]+|\d+)/.exec(message);
  if (!match) return message;
  const code = Number(match[1]);
  const copy = ERROR_COPY[code];
  return copy ? `${copy} (code ${match[1]})` : message;
}
