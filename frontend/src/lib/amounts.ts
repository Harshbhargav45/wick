/**
 * Decimal text ⇄ fixed-point integers, for the console's write forms.
 *
 * Every amount the program takes is an integer: USD at 6dp, or lamports at 9.
 * Parsing through `Number` would silently round anything past ~15 significant
 * digits, and `$0.1 + $0.2` is exactly the class of arithmetic that has no
 * business happening near a number that becomes collateral. So this stays in
 * strings and `bigint` end to end.
 */

export const USD_DECIMALS = 6;
export const LAMPORT_DECIMALS = 9;
export const LAMPORTS_PER_SOL = 1_000_000_000n;

export type ParseResult =
  | { ok: true; value: bigint }
  | { ok: false; error: string };

/**
 * Parse a plain decimal string into a scaled integer.
 *
 * Rejects rather than truncates when the input carries more precision than the
 * scale can hold: silently dropping the tail of `0.0000001` would send a
 * different amount than the one on screen.
 */
export function parseDecimal(input: string, decimals: number): ParseResult {
  const text = input.trim();
  if (text === '') return { ok: false, error: 'Enter an amount.' };
  if (!/^\d*\.?\d*$/.test(text) || text === '.') {
    return { ok: false, error: 'Digits and a single decimal point only.' };
  }

  const [whole = '', fraction = ''] = text.split('.');
  if (fraction.length > decimals) {
    return { ok: false, error: `At most ${decimals} decimal places.` };
  }

  const scaled = BigInt(`${whole || '0'}${fraction.padEnd(decimals, '0')}`);
  if (scaled === 0n) return { ok: false, error: 'Amount must be greater than zero.' };
  return { ok: true, value: scaled };
}

/** USD text → 6dp fixed point, the unit every `parse_amount` handler expects. */
export function parseUsd(input: string): ParseResult {
  return parseDecimal(input, USD_DECIMALS);
}

/** SOL text → lamports, the unit the margin reserve is denominated in. */
export function parseSol(input: string): ParseResult {
  return parseDecimal(input, LAMPORT_DECIMALS);
}

/** Signed decimal, for a position size where negative is short. */
export function parseSignedDecimal(input: string, decimals: number): ParseResult {
  const text = input.trim();
  const negative = text.startsWith('-');
  const parsed = parseDecimal(negative ? text.slice(1) : text, decimals);
  if (!parsed.ok) return parsed;
  return { ok: true, value: negative ? -parsed.value : parsed.value };
}

/** Lamports → SOL for display, without going through a float. */
export function formatSol(lamports: bigint, decimals = 4): string {
  const negative = lamports < 0n;
  const abs = negative ? -lamports : lamports;
  const whole = abs / LAMPORTS_PER_SOL;
  const fraction = (abs % LAMPORTS_PER_SOL).toString().padStart(LAMPORT_DECIMALS, '0');
  const shown = decimals === 0 ? '' : `.${fraction.slice(0, decimals)}`;
  return `${negative ? '-' : ''}${whole.toLocaleString('en-US')}${shown}`;
}
