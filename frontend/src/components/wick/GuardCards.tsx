'use client';

import { useState } from 'react';
import { Activity, Crosshair, ShieldCheck, type LucideIcon } from 'lucide-react';
import { useReveal } from '@/hooks/useWickMotion';
import { latencyStats } from '@/lib/wick-data';
import { Reveal } from './Reveal';
import { cn } from '@/lib/utils';

function HealthDemo({ active }: { active: boolean }) {
  const pct = active ? 34 : 68;
  return (
    <div className="space-y-2">
      <div className="flex justify-between font-mono text-[11px] text-muted-foreground">
        <span>1.00</span>
        <span className="text-foreground">{active ? '1.12' : '1.34'}</span>
        <span>2.00</span>
      </div>
      <div className="relative h-2 overflow-hidden rounded-full bg-muted">
        <div
          className={cn(
            'h-full rounded-full transition-all duration-700 ease-out',
            active ? 'bg-warning' : 'bg-healthy',
          )}
          style={{ width: `${pct + 32}%` }}
        />
        <span
          aria-hidden="true"
          className="absolute top-0 h-full w-px bg-border-strong"
          style={{ left: '15%' }}
        />
      </div>
      <div className="font-mono text-[11px] text-muted-foreground">
        cross-multiplied every tick — equity vs maintenance margin
      </div>
    </div>
  );
}

function TriggerDemo({ active }: { active: boolean }) {
  return (
    <div className="space-y-2">
      <div className="relative h-16 overflow-hidden rounded-md border border-border bg-background/50">
        <div className="absolute inset-x-0 top-1/2 h-px bg-border" />
        <div
          className="absolute inset-y-0 w-px bg-warning transition-all duration-700 ease-out"
          style={{ left: active ? '78%' : '34%' }}
        />
        <div className="absolute inset-y-0 left-[78%] w-px border-l border-dashed border-risk/60" />
        <span className="absolute bottom-1 left-2 font-mono text-[10px] text-muted-foreground">
          health
        </span>
        <span className="absolute bottom-1 right-2 font-mono text-[10px] text-risk">
          1.15 trigger
        </span>
      </div>
      <div
        className={cn(
          'font-mono text-[11px] transition-colors',
          active ? 'text-warning' : 'text-muted-foreground',
        )}
      >
        {active ? '[!] buffer breached — reducing 18%' : 'distance to trigger 0.19'}
      </div>
    </div>
  );
}

const PROOF_STEPS = ['built', 'signed', 'landed'];

function ProofDemo({ active }: { active: boolean }) {
  return (
    <ul className="space-y-2">
      {PROOF_STEPS.map((s, i) => (
        <li
          key={s}
          className={cn(
            'flex items-center gap-2.5 font-mono text-[11.5px] transition-all duration-500',
            active ? 'text-healthy opacity-100' : 'text-muted-foreground opacity-60',
          )}
          style={{ transitionDelay: active ? `${i * 180}ms` : '0ms' }}
        >
          <span
            className={cn(
              'grid h-4 w-4 shrink-0 place-items-center rounded-sm border text-[9px] transition-colors',
              active ? 'border-healthy text-healthy' : 'border-border text-muted-foreground',
            )}
          >
            ✓
          </span>
          reduce order {s}
        </li>
      ))}
      <li className="pt-1 font-mono text-[11px] text-muted-foreground">
        dispatch {latencyStats.p50Us}µs · nonce committed on land
      </li>
    </ul>
  );
}

interface Card {
  key: string;
  icon: LucideIcon;
  title: string;
  body: string;
  demo: (props: { active: boolean }) => React.ReactElement;
}

const CARDS: Card[] = [
  {
    key: 'health',
    icon: Activity,
    title: 'HEALTH',
    body: 'Health is recomputed on every tick from the verified Pyth price, your collateral and position size — in fixed point, cross-multiplied, never divided. The trigger buffer sits above the liquidation threshold, so there is room to move.',
    demo: HealthDemo,
  },
  {
    key: 'trigger',
    icon: Crosshair,
    title: 'TRIGGER',
    body: 'At the threshold the guard acts. A bounded solver finds the smallest partial close that restores the buffer, caps are enforced in USD, and the order is reduce-only by construction — the guard cannot open exposure.',
    demo: TriggerDemo,
  },
  {
    key: 'proof',
    icon: ShieldCheck,
    title: 'PROOF',
    body: 'The nonce only commits once the venue action actually lands, so a failed dispatch never looks like a success. Nothing is silently skipped: if no action fits inside your caps, the guard escalates instead of no-oping.',
    demo: ProofDemo,
  },
];

export function GuardCards() {
  return (
    <section id="guard" className="py-24 sm:py-32">
      <div className="mx-auto max-w-6xl px-4 sm:px-6">
        <Reveal className="max-w-2xl">
          <span className="font-mono text-[11px] tracking-[0.24em] text-primary">
            01 — THE GUARD
          </span>
          <h2 className="mt-4 font-serif text-4xl leading-tight text-foreground sm:text-5xl">
            The guard, explained
          </h2>
          <p className="mt-4 text-muted-foreground">
            A delegated guard reads the position&apos;s health each tick. When it dips, the guard
            responds on the authority regime the venue allows — autonomously where it holds a
            delegation, build-only where it does not.
          </p>
        </Reveal>

        <div className="mt-12 grid gap-4 lg:grid-cols-3">
          {CARDS.map((card, i) => (
            <GuardCard key={card.key} card={card} index={i} />
          ))}
        </div>
      </div>
    </section>
  );
}

function GuardCard({ card, index }: { card: Card; index: number }) {
  const { ref, visible } = useReveal<HTMLDivElement>(0.25);
  const [hovered, setHovered] = useState(false);
  const Demo = card.demo;
  const Icon = card.icon;

  return (
    <div
      ref={ref}
      data-visible={visible}
      tabIndex={0}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setHovered(true)}
      onBlur={() => setHovered(false)}
      style={{ transitionDelay: `${index * 90}ms` }}
      className="reveal group rounded-xl border border-border bg-surface/40 p-6 transition-[border-color,transform,background-color] duration-300 hover:-translate-y-1 hover:border-border-strong hover:bg-surface/70 focus-visible:border-primary"
    >
      <div className="flex items-center gap-2.5">
        <Icon className="h-4 w-4 text-primary transition-transform duration-300 group-hover:scale-110" />
        <h3 className="font-mono text-[11px] tracking-[0.24em] text-muted-foreground">
          {card.title}
        </h3>
      </div>
      <p className="mt-4 text-sm leading-relaxed text-muted-foreground">{card.body}</p>
      <div className="mt-6 border-t border-border pt-5">
        <Demo active={hovered} />
      </div>
    </div>
  );
}
