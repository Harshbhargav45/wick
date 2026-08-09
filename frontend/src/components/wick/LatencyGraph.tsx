'use client';

import { useMemo, useRef, useState } from 'react';
import { headroom, latency, latencyStats } from '@/lib/wick-data';
import { usePrefersReducedMotion, useReveal } from '@/hooks/useWickMotion';
import { cn } from '@/lib/utils';

const W = 900;
const H = 220;
const PAD_L = 52;
const PAD_R = 12;
const PAD_T = 22;
const PAD_B = 22;

function tickLabel(us: number) {
  if (us === 0) return '0';
  return us >= 1000 ? `${(us / 1000).toFixed(1)}ms` : `${us}µs`;
}

export function LatencyGraph({ className }: { className?: string }) {
  const { ref, visible } = useReveal<HTMLDivElement>(0.3);
  const reduced = usePrefersReducedMotion();
  const svgRef = useRef<SVGSVGElement | null>(null);
  const [hover, setHover] = useState<{ x: number; y: number; us: number; i: number } | null>(null);

  const { path, area, points, ticks } = useMemo(() => {
    const samples = latency.samples_us;
    const n = samples.length;
    const yMax = Math.max(500, Math.ceil(latencyStats.maxUs / 500) * 500);
    const step = yMax / 3;
    const x = (i: number) => PAD_L + (i / Math.max(n - 1, 1)) * (W - PAD_L - PAD_R);
    const y = (us: number) => PAD_T + (1 - Math.min(us, yMax) / yMax) * (H - PAD_T - PAD_B);
    const pts = samples.map((us, i) => ({ x: x(i), y: y(us), us, i }));
    const d = pts
      .map((p, i) => `${i === 0 ? 'M' : 'L'}${p.x.toFixed(2)} ${p.y.toFixed(2)}`)
      .join(' ');
    const first = pts[0]!;
    const last = pts[pts.length - 1]!;
    const a = `${d} L${last.x.toFixed(2)} ${H - PAD_B} L${first.x.toFixed(2)} ${H - PAD_B} Z`;
    return {
      path: d,
      area: a,
      points: pts,
      ticks: [0, step, step * 2, yMax].map((us) => ({ us, y: y(us) })),
    };
  }, []);

  const onMove = (e: React.MouseEvent<SVGSVGElement>) => {
    const svg = svgRef.current;
    if (!svg) return;
    const rect = svg.getBoundingClientRect();
    const rel = ((e.clientX - rect.left) / rect.width) * W;
    let nearest = points[0]!;
    for (const p of points) if (Math.abs(p.x - rel) < Math.abs(nearest.x - rel)) nearest = p;
    setHover(nearest);
  };

  return (
    <div ref={ref} className={cn('relative', className)}>
      <svg
        ref={svgRef}
        viewBox={`0 0 ${W} ${H}`}
        className="w-full touch-none"
        role="img"
        aria-label={`Recorded dispatch latency: p50 ${latencyStats.p50Us} microseconds, p99 ${latencyStats.p99Us} microseconds across ${latencyStats.samples} dispatches.`}
        onMouseMove={onMove}
        onMouseLeave={() => setHover(null)}
      >
        <defs>
          <linearGradient id="wick-lat" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--primary)" stopOpacity="0.28" />
            <stop offset="100%" stopColor="var(--primary)" stopOpacity="0" />
          </linearGradient>
        </defs>

        {ticks.map((t) => (
          <g key={t.us}>
            <line
              x1={PAD_L}
              x2={W - PAD_R}
              y1={t.y}
              y2={t.y}
              stroke="var(--border)"
              strokeWidth="1"
              strokeDasharray="2 6"
            />
            <text
              x={PAD_L - 8}
              y={t.y + 3.5}
              textAnchor="end"
              className="fill-muted-foreground font-mono"
              fontSize="10"
            >
              {tickLabel(t.us)}
            </text>
          </g>
        ))}

        <line
          x1={PAD_L}
          x2={W - PAD_R}
          y1={PAD_T - 10}
          y2={PAD_T - 10}
          stroke="var(--healthy)"
          strokeWidth="1"
          strokeDasharray="6 6"
          opacity="0.8"
        />
        <text x={PAD_L + 6} y={PAD_T - 14} className="fill-healthy font-mono" fontSize="10">
          {latencyStats.targetMs}ms target — off scale, {headroom}× headroom
        </text>

        <path
          d={area}
          fill="url(#wick-lat)"
          opacity={visible ? 1 : 0}
          style={{ transition: 'opacity 900ms ease' }}
        />
        <path
          d={path}
          fill="none"
          stroke="var(--primary)"
          strokeWidth="1.5"
          strokeLinejoin="round"
          pathLength={1}
          strokeDasharray={reduced ? undefined : 1}
          strokeDashoffset={reduced ? undefined : visible ? 0 : 1}
          style={
            reduced
              ? undefined
              : { transition: 'stroke-dashoffset 1600ms cubic-bezier(0.22,1,0.36,1)' }
          }
        />

        {hover ? (
          <g>
            <line
              x1={hover.x}
              x2={hover.x}
              y1={PAD_T}
              y2={H - PAD_B}
              stroke="var(--border-strong)"
              strokeWidth="1"
            />
            <circle cx={hover.x} cy={hover.y} r="3" fill="var(--primary)" />
          </g>
        ) : null}
      </svg>

      {hover ? (
        <div
          className="pointer-events-none absolute -translate-x-1/2 rounded-md border border-border bg-popover px-2.5 py-1.5 font-mono text-[11px] text-popover-foreground shadow-lg"
          style={{ left: `${(hover.x / W) * 100}%`, top: 0 }}
        >
          <span className="text-primary">{hover.us}µs</span>
          <span className="ml-2 text-muted-foreground">#{hover.i + 1}</span>
        </div>
      ) : null}
    </div>
  );
}
