import dataset from '../../public/latency-samples.json';

export interface LatencyDataset {
  lanes: { l1_slot_ms: number; sub_50ms_target_us: number };
  samples_us: number[];
  summary_us: { min: number; p50: number; p99: number; max: number };
  note?: string;
}

/**
 * The measured dispatch latencies, recorded by `program/tests/latency_bench.rs`
 * and regenerated into `public/latency-samples.json`. Every latency number the
 * site shows resolves back to this file — there are no hand-written figures.
 */
export const latency = dataset as LatencyDataset;

export const latencyStats = {
  samples: latency.samples_us.length,
  slotMs: latency.lanes.l1_slot_ms,
  targetMs: latency.lanes.sub_50ms_target_us / 1000,
  minUs: latency.summary_us.min,
  p50Us: latency.summary_us.p50,
  p99Us: latency.summary_us.p99,
  maxUs: latency.summary_us.max,
  note: latency.note,
};

/** Headroom of the measured p50 against the sub-50ms target lane. */
export const headroom = Math.round(
  latency.lanes.sub_50ms_target_us / latency.summary_us.p50,
);

export const ecosystem = ['DRIFT', 'JUPITER', 'PYTH', 'MAGICBLOCK', 'SOLANA'];

/**
 * MagicBlock's Ephemeral Rollup is what the guard delegates its PDA into
 * (§8.6, `program/src/delegation.rs`) so the Drift venue can run autonomous
 * checks inside a slot. It is a real dependency, not a logo — the delegate /
 * commit / undelegate instructions are discriminators 4–6.
 */
export const magicblock = {
  name: 'MagicBlock',
  href: 'https://magicblock.gg',
  docsHref: 'https://docs.magicblock.gg',
  tagline: 'Ephemeral Rollups',
};

/**
 * The rail on the landing page. Each card names something the guard actually
 * does and where it is enforced, so the row stays a summary of the system
 * rather than decoration.
 */
export const stackCards = [
  {
    key: 'er',
    tag: 'MAGICBLOCK',
    title: 'Ephemeral Rollup',
    body: 'The guard PDA delegates into the ER so Drift checks run inside a slot, then commits back to L1.',
  },
  {
    key: 'drift',
    tag: 'DRIFT',
    title: 'Autonomous',
    body: 'The guard holds a delegation and signs the reduce-only CPI itself. Nonce commits on land.',
  },
  {
    key: 'jupiter',
    tag: 'JUPITER',
    title: 'Co-signed',
    body: 'No delegation, so the guard builds the instruction and waits. You sign it on L1.',
  },
  {
    key: 'pyth',
    tag: 'PYTH',
    title: 'Verified price',
    body: 'Every tick prices against a PriceUpdateV2 inside the staleness and confidence gate.',
  },
  {
    key: 'caps',
    tag: 'POLICY',
    title: 'USD caps',
    body: 'Per-action and daily caps are checked in USD notional, and the daily total accumulates.',
  },
  {
    key: 'solver',
    tag: 'SOLVER',
    title: 'Smallest close',
    body: 'A bounded binary search finds the least you have to close to get back above the buffer.',
  },
  {
    key: 'killswitch',
    tag: 'ROUTECONFIG',
    title: 'Kill switch',
    body: 'Every state-mutating instruction checks the pause flag — and rejects if it is missing.',
  },
];

/**
 * The problem the Ephemeral Rollup solves, stated as the gap it closes.
 *
 * The numbers are derived, not written: `slotMs` is the L1 lane from the
 * recorded dataset and `p50Us` is the measured dispatch. Every claim here maps
 * to a real instruction — Delegate=4, Commit=6, CommitAndUndelegate=5 in
 * `program/src/instruction.rs`, handled in `program/src/delegation.rs`.
 */
export const erProblem = {
  /** Without the ER: the guard can only act once per L1 slot. */
  l1: {
    label: 'ON L1 ALONE',
    headline: `One check per ~${latencyStats.slotMs}ms slot`,
    body: 'A liquidation cascade moves inside a slot. If the guard only wakes when the next block lands, the price it read is already history and the position is already gone.',
    marks: [
      'price read is a block old before it is used',
      'one tick per slot, no matter how fast the move',
      'every check pays L1 fees and contends for blockspace',
    ],
  },
  /** With the ER: many checks per slot, committed back to L1. */
  er: {
    label: 'DELEGATED TO THE ER',
    headline: `${latencyStats.p50Us}µs per check, measured`,
    body: 'The guard PDA delegates into a MagicBlock Ephemeral Rollup. Checks run on delegated state at ER speed, and the state commits back to L1 — so the guard reacts inside the window the cascade actually happens in.',
    marks: [
      `p50 dispatch ${latencyStats.p50Us}µs over ${latencyStats.samples} recorded runs`,
      'many ticks inside a single L1 slot',
      'state commits back to L1; ownership reverts on undelegate',
    ],
  },
};

/**
 * The delegation round trip (§8.6). One entry per real instruction, in the
 * order a session runs them.
 */
export const erFlow = [
  {
    key: 'delegate',
    ix: 'Delegate',
    disc: 4,
    label: 'Guard delegates into the ER',
    note: 'owner signs · PDA moves to the validator',
  },
  {
    key: 'tick',
    ix: 'OnPriceTick',
    disc: 7,
    label: 'Ticks run on delegated state',
    note: `${latencyStats.p50Us}µs p50 · price → health → caps → dispatch`,
  },
  {
    key: 'commit',
    ix: 'Commit',
    disc: 6,
    label: 'State commits back to L1',
    note: 'guard stays delegated',
  },
  {
    key: 'undelegate',
    ix: 'CommitAndUndelegate',
    disc: 5,
    label: 'Session ends, ownership reverts',
    note: 'final commit · PDA returns to the program',
  },
];

/** Short strings for the footer rail. */
export const footerTicker = [
  'reduce-only by construction',
  'nonce commits on land',
  'fixed point · never divides',
  'delegated to MagicBlock ER',
  'pyth pull oracle · full verification',
  'per-action + daily USD caps',
  'kill switch fails closed',
  'you hold the keys',
];


/**
 * The guard's critical path (§7.2), one stage per instruction step. These are
 * the real stages `on_price_tick` runs, in order — not a marketing sequence.
 */
export const mechanism = [
  { key: 'price', label: 'Read Pyth price', note: 'PriceUpdateV2, Full-verified' },
  { key: 'staleness', label: 'Check staleness', note: '3 stale ticks → degraded' },
  { key: 'health', label: 'Compute health', note: 'cross-multiplied, no division' },
  { key: 'nonce', label: 'Check nonce', note: 'monotonic, steps by one' },
  { key: 'caps', label: 'Enforce caps', note: 'per-action + daily USD' },
  { key: 'select', label: 'Select action', note: 'TP → top-up → partial close' },
  { key: 'dispatch', label: 'Dispatch', note: 'autonomous, or build for owner' },
  { key: 'commit', label: 'Commit nonce', note: 'only once the action lands' },
];

/** Latency buckets over the recorded sample, in µs. */
export function latencyBuckets() {
  const bounds = [200, 300, 500, Infinity];
  const labels = ['<200µs', '200–300µs', '300–500µs', '500µs+'];
  const counts = new Array(bounds.length).fill(0);

  for (const us of latency.samples_us) {
    const idx = bounds.findIndex((b) => us < b);
    counts[idx === -1 ? bounds.length - 1 : idx] += 1;
  }

  const total = latency.samples_us.length || 1;
  return labels.map((label, i) => ({
    label,
    pct: Math.round((counts[i] / total) * 100),
  }));
}
