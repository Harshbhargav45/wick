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

export const ecosystem = ['DRIFT', 'JUPITER', 'PYTH', 'SOLANA'];

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
