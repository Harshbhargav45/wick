/**
 * Margin reserve builders — Gap 3's cranker half.
 *
 * These assert the wire format against `program/tests/margin_wallet.rs`, which
 * is itself verified on the real SBF VM. A builder that disagrees with the
 * program produces a transaction that fails in preflight — recoverable — but the
 * amount encoding is the dangerous part: a wrong-endian or float-rounded amount
 * moves the wrong quantity of real value and succeeds.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { LAMPORTS_PER_SOL, PublicKey } from "@solana/web3.js";
import {
  buildFundIx,
  buildInitIx,
  buildWithdrawIx,
  decodeWallet,
  solToLamports,
} from "../src/margin-wallet.mjs";
import { IX } from "../src/guard-layout.mjs";

const PROGRAM_ID = new PublicKey("11111111111111111111111111111112");
const WALLET = new PublicKey("11111111111111111111111111111113");
const GUARD = new PublicKey("11111111111111111111111111111114");
const OWNER = new PublicKey("11111111111111111111111111111115");
const CO_AUTHORITY = new PublicKey("11111111111111111111111111111116");
const ROUTE_CONFIG = new PublicKey("11111111111111111111111111111117");
const SYSTEM_PROGRAM = "11111111111111111111111111111111";

const BASE = {
  programId: PROGRAM_ID,
  wallet: WALLET,
  guard: GUARD,
  owner: OWNER,
  routeConfig: ROUTE_CONFIG,
};

function readU128LE(d, off) {
  let v = 0n;
  for (let i = 15; i >= 0; i--) v = (v << 8n) | BigInt(d[off + i]);
  return v;
}

test("InitMarginWallet carries the bump and the system program", () => {
  const ix = buildInitIx({ ...BASE, bump: 254 });
  assert.equal(ix.data.length, 2);
  assert.equal(ix.data[0], IX.InitMarginWallet);
  assert.equal(ix.data[0], 14);
  assert.equal(ix.data[1], 254, "the program re-derives the PDA from this bump");

  // Order is verified against program/tests/margin_wallet.rs:156-164.
  assert.equal(ix.keys.length, 7);
  assert.equal(ix.keys[0].pubkey.toBase58(), WALLET.toBase58());
  assert.equal(ix.keys[0].isWritable, true);
  assert.equal(ix.keys[1].pubkey.toBase58(), GUARD.toBase58());
  assert.equal(ix.keys[1].isWritable, true, "init stamps margin_wallet_bump on the guard");
  assert.equal(ix.keys[2].isSigner, true, "the owner authorizes");
  assert.equal(ix.keys[3].isWritable, true, "the payer is debited for rent");
  // Without this the CreateAccount CPI fails: the callee must be in the
  // transaction's account list.
  assert.equal(ix.keys[6].pubkey.toBase58(), SYSTEM_PROGRAM);
});

test("FundMarginWallet is discriminator 15 and a 16-byte little-endian amount", () => {
  const ix = buildFundIx({ ...BASE, lamports: 2n * BigInt(LAMPORTS_PER_SOL) });
  assert.equal(ix.data.length, 17, "the program rejects any other payload length");
  assert.equal(ix.data[0], IX.FundMarginWallet);
  assert.equal(ix.data[0], 15);
  assert.equal(readU128LE(ix.data, 1), 2_000_000_000n);
  // The guard is only read here — the reserve's balance lives on the wallet.
  assert.equal(ix.keys[1].isWritable, false);
  assert.equal(ix.keys[2].isWritable, true, "the owner is debited");
  assert.equal(ix.keys.length, 6);
  assert.equal(ix.keys[5].pubkey.toBase58(), SYSTEM_PROGRAM);
});

test("WithdrawMarginWallet requires two signers and no system program", () => {
  const ix = buildWithdrawIx({
    ...BASE,
    coAuthority: CO_AUTHORITY,
    lamports: 500_000_000n,
  });
  assert.equal(ix.data[0], IX.WithdrawMarginWallet);
  assert.equal(ix.data[0], 16);
  assert.equal(readU128LE(ix.data, 1), 500_000_000n);

  assert.equal(ix.keys.length, 6);
  assert.equal(ix.keys[2].isSigner, true, "owner");
  assert.equal(ix.keys[3].pubkey.toBase58(), CO_AUTHORITY.toBase58());
  assert.equal(
    ix.keys[3].isSigner,
    true,
    "§8.5 makes withdrawal 2-of-2; a missing signer flag is refused on chain"
  );
  // The reserve is program-owned, so the program moves lamports by writing
  // both accounts directly — no CPI, hence no system program.
  assert.ok(
    !ix.keys.some((k) => k.pubkey.toBase58() === SYSTEM_PROGRAM),
    "withdrawal does not CPI"
  );
});

test("SOL is converted exactly, without floating-point drift", () => {
  assert.equal(solToLamports("1"), 1_000_000_000n);
  assert.equal(solToLamports("0.000000001"), 1n);
  assert.equal(solToLamports("2.5"), 2_500_000_000n);
  // 0.3 is not representable in binary floating point; `0.3 * 1e9` is
  // 300000000.00000006 and would truncate or round into the amount.
  assert.equal(solToLamports("0.3"), 300_000_000n);
  assert.equal(solToLamports("123.456789012"), 123_456_789_012n);
});

test("an amount that is not a positive decimal is refused before it is sent", () => {
  for (const bad of ["", "abc", "-1", "1e9", "0", "0.0", " ", "1.2.3", undefined]) {
    assert.throws(
      () => solToLamports(bad),
      /amount must be|not representable/,
      `"${bad}" should not become an amount`
    );
  }
  assert.throws(() => solToLamports("1.0000000001"), /not representable/);
});

test("the wallet decoder reads the fields the program writes", () => {
  const d = Buffer.alloc(81);
  d[0] = 1;
  OWNER.toBuffer().copy(d, 1);
  CO_AUTHORITY.toBuffer().copy(d, 33);
  let v = 7_000_000_000n;
  for (let i = 0; i < 16; i++) {
    d[65 + i] = Number(v & 0xffn);
    v >>= 8n;
  }
  const ws = decodeWallet(d);
  assert.equal(ws.version, 1);
  assert.equal(ws.owner.toBase58(), OWNER.toBase58());
  assert.equal(ws.coAuthority.toBase58(), CO_AUTHORITY.toBase58());
  assert.equal(ws.balance, 7_000_000_000n);
});

test("a wrong-length account is refused rather than decoded as a reserve", () => {
  // The guard is 416 bytes and lives at a nearby PDA; decoding it as a wallet
  // would report a nonsense balance for a real account.
  assert.throws(() => decodeWallet(Buffer.alloc(416)), /not a margin wallet: len=416/);
  assert.throws(() => decodeWallet(Buffer.alloc(0)), /len=0/);
  assert.throws(() => decodeWallet(undefined), /len=undefined/);
});
