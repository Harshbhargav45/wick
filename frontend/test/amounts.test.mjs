import { strict as assert } from 'node:assert';
import { describe, it } from 'node:test';

import {
  formatSol,
  parseDecimal,
  parseSignedDecimal,
  parseSol,
  parseUsd,
} from '../.test-build/amounts.js';

/**
 * These forms take free text and produce a number the program will spend, so the
 * cases that matter are the ones where a lenient parser would send a *different*
 * amount than the one on screen rather than reject it.
 */
describe('amount parsing', () => {
  it('scales USD to 6dp fixed point', () => {
    assert.equal(parseUsd('1').ok && parseUsd('1').value, 1_000_000n);
    assert.equal(parseUsd('0.5').ok && parseUsd('0.5').value, 500_000n);
    assert.equal(parseUsd('1234.567891').ok && parseUsd('1234.567891').value, 1_234_567_891n);
  });

  it('scales SOL to lamports', () => {
    assert.equal(parseSol('1').ok && parseSol('1').value, 1_000_000_000n);
    assert.equal(parseSol('0.000000001').ok && parseSol('0.000000001').value, 1n);
  });

  it('rejects excess precision instead of truncating it', () => {
    // Truncating would send $0.000001 for an input reading 0.0000019 — a
    // different amount than the one the owner typed and can see.
    const r = parseUsd('0.0000019');
    assert.equal(r.ok, false);
    assert.match(r.ok ? '' : r.error, /at most 6 decimal places/i);
  });

  it('rejects zero, empty and non-numeric input', () => {
    for (const bad of ['', '   ', '.', '0', '0.000000', 'abc', '1e6', '1,000', '--1']) {
      assert.equal(parseUsd(bad).ok, false, `expected ${JSON.stringify(bad)} to be rejected`);
    }
  });

  it('never loses precision past the float-safe range', () => {
    // 2^53 + 1 in whole USD. Through `Number` this would land on an even value.
    const text = '9007199254740993';
    const r = parseUsd(text);
    assert.equal(r.ok, true);
    assert.equal(r.ok && r.value, 9_007_199_254_740_993_000_000n);
  });

  it('parses signed sizes, where negative is short', () => {
    const short = parseSignedDecimal('-2.5', 6);
    assert.equal(short.ok && short.value, -2_500_000n);
    const long = parseSignedDecimal('2.5', 6);
    assert.equal(long.ok && long.value, 2_500_000n);
    // A bare minus is not a number.
    assert.equal(parseSignedDecimal('-', 6).ok, false);
  });

  it('honours the requested scale', () => {
    assert.equal(parseDecimal('1.5', 2).ok && parseDecimal('1.5', 2).value, 150n);
    assert.equal(parseDecimal('1.555', 2).ok, false);
  });

  it('formats lamports without going through a float', () => {
    assert.equal(formatSol(1_000_000_000n), '1.0000');
    assert.equal(formatSol(1_500_000_000n), '1.5000');
    assert.equal(formatSol(1n, 9), '0.000000001');
    assert.equal(formatSol(0n), '0.0000');
    // The rent portion of a reserve is shown as a negative nowhere, but the
    // formatter is the one place that would silently print "-0".
    assert.equal(formatSol(-1_500_000_000n), '-1.5000');
  });
});
