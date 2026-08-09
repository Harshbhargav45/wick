'use client';

import { useEffect, useState } from 'react';
import styles from './Console.module.css';

export interface LatencyDataset {
  lanes: { l1_slot_ms: number; sub_50ms_target_us: number };
  samples_us: number[];
  summary_us: { min: number; p50: number; p99: number; max: number };
  note?: string;
}

const W = 560;
const H = 160;
const PAD_L = 48;
const PAD_R = 12;
const PAD_T = 16;
const PAD_B = 24;

function fmt(us: number): string {
  return us >= 1000 ? `${(us / 1000).toFixed(2)}ms` : `${us.toFixed(0)}µs`;
}

export function LatencyChart() {
  const [ds, setDs] = useState<LatencyDataset | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetch('/latency-samples.json')
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(`${r.status}`))))
      .then(setDs)
      .catch((e) => setError(e.message));
  }, []);

  if (error) {
    return (
      <div className={styles.latencyCard}>
        <div className={styles.healthLabel}>Autonomous dispatch latency</div>
        <div className={styles.statSecondary}>Dataset unavailable ({error})</div>
      </div>
    );
  }
  if (!ds) {
    return (
      <div className={styles.latencyCard}>
        <div className={styles.healthLabel}>Autonomous dispatch latency</div>
        <div className={styles.statSecondary}>loading samples…</div>
      </div>
    );
  }

  const samples = ds.samples_us;
  const summary = ds.summary_us;
  // Log-ish y-scale: draw 0..max but cap axis at a clean 10ms so the μs
  // cluster stays visible against the 50ms target / 400ms baseline lines.
  const maxUs = Math.max(...samples, summary.max, ds.lanes.sub_50ms_target_us);
  const yMax = Math.max(maxUs, 10_000);
  const x = (i: number) => PAD_L + (i / Math.max(samples.length - 1, 1)) * (W - PAD_L - PAD_R);
  const y = (us: number) => PAD_T + (1 - us / yMax) * (H - PAD_T - PAD_B);

  const band50 = y(0) - y(ds.lanes.sub_50ms_target_us);
  const lineL1 = y(ds.lanes.l1_slot_ms * 1000);

  const path = samples
    .map((us, i) => `${i === 0 ? 'M' : 'L'}${x(i).toFixed(1)},${y(us).toFixed(1)}`)
    .join(' ');
  const area = `${path} L${(W - PAD_R).toFixed(1)},${y(0).toFixed(1)} L${PAD_L},${y(0).toFixed(1)} Z`;

  return (
    <div className={styles.latencyCard}>
      <div className={styles.latencyHeader}>
        <div className={styles.healthLabel}>Autonomous dispatch latency</div>
        <div className={styles.latencyClaim}>
          p50 <span className="mono">{fmt(summary.p50)}</span>{' '}
          <span className={styles.statSecondary}>vs L1 slot ~{ds.lanes.l1_slot_ms}ms</span>
        </div>
      </div>

      <svg
        viewBox={`0 0 ${W} ${H}`}
        width="100%"
        role="img"
        aria-label="Latency of autonomous guard dispatch vs Solana L1 baseline"
      >
        <defs>
          <linearGradient id="latFill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--accent-ember)" stopOpacity="0.35" />
            <stop offset="100%" stopColor="var(--accent-ember)" stopOpacity="0.02" />
          </linearGradient>
        </defs>

        {/* gridlines */}
        {Array.from(new Set([0, 5, 10, ds.lanes.sub_50ms_target_us / 1000, yMax / 1000])).map((ms) => {
          const gy = y(ms * 1000);
          return (
            <g key={ms}>
              <line
                x1={PAD_L}
                x2={W - PAD_R}
                y1={gy}
                y2={gy}
                stroke="var(--border-hairline)"
                strokeWidth={1}
              />
              <text x={PAD_L - 6} y={gy + 3} textAnchor="end" fontSize={9} fill="var(--text-tertiary)" className="mono">
                {ms >= 1000 ? `${ms / 1000}ms` : ms === 0 ? '0' : `${ms}ms`}
              </text>
            </g>
          );
        })}

        {/* sub-50ms band */}
        <rect x={PAD_L} y={y(ds.lanes.sub_50ms_target_us)} width={W - PAD_L - PAD_R} height={band50} fill="var(--state-safe)" opacity={0.06} />
        <line x1={PAD_L} x2={W - PAD_R} y1={y(ds.lanes.sub_50ms_target_us)} y2={y(ds.lanes.sub_50ms_target_us)} stroke="var(--state-safe)" strokeWidth={1} strokeDasharray="3 3" />
        <text x={PAD_L + 4} y={y(ds.lanes.sub_50ms_target_us) - 4} fontSize={9} fill="var(--state-safe)" className="mono">
          50ms target
        </text>

        {/* L1 baseline */}
        <line x1={PAD_L} x2={W - PAD_R} y1={lineL1} y2={lineL1} stroke="var(--state-danger)" strokeWidth={1} strokeDasharray="6 3" />
        <text x={W - PAD_R - 4} y={lineL1 - 4} textAnchor="end" fontSize={9} fill="var(--state-danger)" className="mono">
          L1 ~{ds.lanes.l1_slot_ms}ms
        </text>

        {/* samples */}
        <path d={area} fill="url(#latFill)" />
        <path d={path} fill="none" stroke="var(--accent-ember)" strokeWidth={1.5} />
      </svg>

      <div className={styles.latencyLegend}>
        <span className={styles.statSecondary}>min {fmt(summary.min)}</span>
        <span className={styles.statSecondary}>p50 {fmt(summary.p50)}</span>
        <span className={styles.statSecondary}>p99 {fmt(summary.p99)}</span>
        <span className={styles.statSecondary}>max {fmt(summary.max)}</span>
        <span className={styles.statSecondary}>n={samples.length}</span>
      </div>
      {ds.note && <div className={styles.statSecondary}>{ds.note}</div>}
    </div>
  );
}