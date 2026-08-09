'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { RotateCw } from 'lucide-react';
import { usePrefersReducedMotion } from '@/hooks/useWickMotion';
import { latencyStats } from '@/lib/wick-data';
import { cn } from '@/lib/utils';

type LineKind = 'cmd' | 'ok' | 'warn' | 'muted' | 'value';

interface Line {
  text: string;
  kind: LineKind;
  wait?: number;
}

const SEQUENCE: Line[] = [
  { text: 'wick guard --market SOL-PERP --venue drift', kind: 'cmd' },
  { text: 'authority delegated · reduce-only · buffer 1.15', kind: 'muted', wait: 260 },
  { text: 'watching slot stream', kind: 'cmd', wait: 300 },
  { text: 'slot 341208119   health 1.34', kind: 'value', wait: 320 },
  { text: 'position healthy', kind: 'ok', wait: 240 },
  { text: 'pyth price update verified', kind: 'cmd', wait: 380 },
  { text: 'slot 341208204   health 1.27', kind: 'value', wait: 300 },
  { text: 'solving partial-close fraction', kind: 'cmd', wait: 300 },
  { text: 'slot 341208288   health 1.12', kind: 'value', wait: 300 },
  { text: 'trigger buffer breached', kind: 'warn', wait: 220 },
  { text: 'guard triggered', kind: 'cmd', wait: 260 },
  { text: 'reducing exposure by 18% (reduce-only)', kind: 'cmd', wait: 300 },
  { text: 'drift place_perp_order signed', kind: 'ok', wait: 260 },
  { text: 'cpi dispatched', kind: 'ok', wait: 240 },
  { text: 'action landed · nonce committed', kind: 'ok', wait: 280 },
  { text: 'health restored 1.12 → 1.34', kind: 'value', wait: 260 },
];

const PREFIX: Record<LineKind, string> = {
  cmd: '>',
  ok: '[✓]',
  warn: '[!]',
  muted: ' ',
  value: ' ',
};

const TONE: Record<LineKind, string> = {
  cmd: 'text-foreground',
  ok: 'text-healthy',
  warn: 'text-warning',
  muted: 'text-muted-foreground',
  value: 'text-primary',
};

const TYPE_SPEED = 16;

export function HeroTerminal() {
  const reduced = usePrefersReducedMotion();
  const [index, setIndex] = useState(0);
  const [typed, setTyped] = useState('');
  const [runId, setRunId] = useState(0);
  const paused = useRef(false);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  const replay = useCallback(() => {
    setIndex(0);
    setTyped('');
    setRunId((r) => r + 1);
  }, []);

  useEffect(() => {
    if (reduced) return;

    let cancelled = false;
    const timers: ReturnType<typeof setTimeout>[] = [];

    const sleep = (ms: number) =>
      new Promise<void>((resolve) => {
        const wait = () => {
          if (paused.current) {
            timers.push(setTimeout(wait, 120));
          } else {
            timers.push(setTimeout(resolve, ms));
          }
        };
        wait();
      });

    const run = async () => {
      await sleep(500);
      for (let i = 0; i < SEQUENCE.length; i++) {
        if (cancelled) return;
        const line = SEQUENCE[i]!;
        await sleep(line.wait ?? 220);
        for (let c = 1; c <= line.text.length; c++) {
          if (cancelled) return;
          setTyped(line.text.slice(0, c));
          await sleep(TYPE_SPEED);
        }
        if (cancelled) return;
        setTyped('');
        setIndex(i + 1);
      }
      await sleep(3600);
      if (!cancelled) replay();
    };

    void run();
    return () => {
      cancelled = true;
      timers.forEach(clearTimeout);
    };
  }, [runId, reduced, replay]);

  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [index, typed, reduced]);

  const reached = reduced ? SEQUENCE.length : index;
  const visible = SEQUENCE.slice(0, reached);
  const active = reduced || reached >= SEQUENCE.length ? null : SEQUENCE[reached];
  const phase =
    reached >= 14 ? 'CONFIRM' : reached >= 10 ? 'ACT' : reached >= 9 ? 'DETECT' : 'WATCH';

  return (
    <div
      className="group relative overflow-hidden rounded-xl border border-border bg-surface/80 shadow-[0_40px_120px_-60px_rgba(0,0,0,0.9)] backdrop-blur-sm"
      onMouseEnter={() => {
        paused.current = true;
      }}
      onMouseLeave={() => {
        paused.current = false;
      }}
    >
      <div className="flex items-center justify-between gap-3 border-b border-border bg-surface-raised/60 px-4 py-2.5">
        <div className="flex min-w-0 items-center gap-2.5">
          <span className="flex shrink-0 gap-1.5" aria-hidden="true">
            <span className="h-2 w-2 rounded-full bg-border-strong" />
            <span className="h-2 w-2 rounded-full bg-border-strong" />
            <span className="h-2 w-2 rounded-full bg-border-strong" />
          </span>
          <span className="truncate font-mono text-[11px] tracking-[0.2em] text-muted-foreground">
            WICK // AUTONOMOUS GUARD
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-3">
          <span className="font-mono text-[10px] tracking-[0.24em] text-primary">{phase}</span>
          <button
            type="button"
            onClick={replay}
            aria-label="Replay guard sequence"
            className="rounded p-1 text-muted-foreground opacity-0 transition-all hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100"
          >
            <RotateCw className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>

      <div
        ref={scrollRef}
        aria-live="polite"
        className="flex h-[300px] flex-col justify-end overflow-hidden px-4 py-3.5 font-mono text-[12.5px] leading-[1.85] sm:h-[340px] sm:text-[13px]"
      >
        {visible.map((line, i) => (
          <div key={`${runId}-${i}`} className={cn('flex gap-2.5 animate-fade-in', TONE[line.kind])}>
            <span className="w-6 shrink-0 text-right opacity-60">{PREFIX[line.kind]}</span>
            <span className="min-w-0 break-words">{line.text}</span>
          </div>
        ))}
        {active ? (
          <div className={cn('flex gap-2.5', TONE[active.kind])}>
            <span className="w-6 shrink-0 text-right opacity-60">{PREFIX[active.kind]}</span>
            <span className="min-w-0 break-words">
              {typed}
              <span
                aria-hidden="true"
                className="ml-0.5 inline-block h-[1em] w-[7px] translate-y-[2px] bg-primary animate-caret"
              />
            </span>
          </div>
        ) : null}
      </div>

      <dl className="grid grid-cols-3 border-t border-border bg-surface-raised/40 font-mono text-[11px]">
        {[
          ['dispatch', `${latencyStats.p50Us}µs`],
          ['venue', 'Drift'],
          ['status', 'protected'],
        ].map(([k, v], i) => (
          <div key={k} className={cn('px-4 py-3', i > 0 && 'border-l border-border')}>
            <dt className="tracking-[0.18em] text-muted-foreground">{k}</dt>
            <dd className={cn('mt-1', k === 'status' ? 'text-healthy' : 'text-foreground')}>{v}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
