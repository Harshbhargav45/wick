import { strict as assert } from 'node:assert';
import { afterEach, describe, it } from 'node:test';

import { explorerAccountUrl, explorerTxUrl } from '../.test-build/explorer.js';

const ENV = 'NEXT_PUBLIC_SOLANA_CLUSTER';
const saved = process.env[ENV];

afterEach(() => {
  if (saved === undefined) delete process.env[ENV];
  else process.env[ENV] = saved;
});

describe('explorer urls', () => {
  it('uses the bare url on mainnet', () => {
    process.env[ENV] = 'mainnet-beta';
    assert.equal(explorerAccountUrl('abc'), 'https://explorer.solana.com/address/abc');
    assert.equal(explorerTxUrl('sig'), 'https://explorer.solana.com/tx/sig');
  });

  it('treats the mainnet alias as mainnet', () => {
    process.env[ENV] = 'mainnet';
    assert.equal(explorerAccountUrl('abc'), 'https://explorer.solana.com/address/abc');
  });

  it('appends the cluster everywhere else', () => {
    process.env[ENV] = 'devnet';
    assert.equal(explorerAccountUrl('abc'), 'https://explorer.solana.com/address/abc?cluster=devnet');
    assert.equal(explorerTxUrl('sig'), 'https://explorer.solana.com/tx/sig?cluster=devnet');
  });

  it('defaults to devnet when the cluster is unset', () => {
    delete process.env[ENV];
    assert.match(explorerTxUrl('sig'), /[?&]cluster=devnet$/);
  });

  it('encodes values rather than pasting them into the path', () => {
    process.env[ENV] = 'devnet';
    assert.match(explorerAccountUrl('a/b?c'), /address\/a%2Fb%3Fc\?cluster=devnet$/);
  });
});
