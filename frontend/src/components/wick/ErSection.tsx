'use client';

import { erFlow, erProblem, latencyStats, magicblock } from '@/lib/wick-data';
import { usePrefersReducedMotion, useReveal } from '@/hooks/useWickMotion';
import { PoweredByMagicBlock } from './PoweredByMagicBlock';
import { Reveal } from './Reveal';
import { cn } from '@/lib/utils';

/** How many p50 dispatches fit inside one L1 slot. The whole argument, as a number. */
const TICKS_PER_SLOT = Math.floor((latencyStats.slotMs * 1000) / latencyStats.p50Us);

/**
 * The problem statement: a liquidation cascade moves faster than an L1 slot, so
 * a guard that can only act once per block is always reading history. The two
 * panels put the lanes side by side, and the flow below walks the four real
 * delegation instructions.
 */
export function ErSection() {
  const { ref, visible } = useReveal<HTMLDivElement>(0.2);
  const reduced = usePrefersReducedMotion();

  return (
    <section id="magicblock" className="border-t border-border/70 bg-surface/20 py-20 sm:py-32">
      <div className="mx-auto max-w-6xl px-4 sm:px-6">
        <Reveal className="max-w-2xl">
          <span className="font-mono text-[11px] tracking-[0.24em] text-primary">
            03 — MAGICBLOCK
          </span>
          <h2 className="mt-4 font-serif text-3xl leading-tight text-foreground sm:text-5xl">
            A cascade does not wait for the next block
          </h2>
          <p className="mt-4 text-sm leading-relaxed text-muted-foreground sm:text-base">
            Liquidations happen inside a slot. A guard that can only act once per block is reading a
            price that has already moved. Wick delegates its guard PDA into a{' '}
            <a
              href={magicblock.href}
              target="_blank"
              rel="noreferrer"
              className="text-foreground underline decoration-border-strong underline-offset-4 transition-colors hover:text-primary hover:decoration-primary"
            >
              MagicBlock Ephemeral Rollup
            </a>{' '}
            so it can check, decide and dispatch many times inside the window the move is actually
            happening in.
          </p>
          <PoweredByMagicBlock variant="feature" className="mt-6" />
        </Reveal>

        <div ref={ref} className="mt-10 grid gap-4 sm:mt-14 lg:grid-cols-2">
          <Lane
            tone="risk"
            visible={visible}
            delay={0}
            {...erProblem.l1}
            bar={{ fill: 4, caption: `1 check per ${latencyStats.slotMs}ms slot` }}
          />
          <Lane
            tone="healthy"
            visible={visible}
            delay={140}
            {...erProblem.er}
            bar={{
              fill: 100,
              caption: `≈${TICKS_PER_SLOT.toLocaleString()} checks fit in the same slot`,
            }}
          />
        </div>

        {/* The round trip. Each step is a real instruction discriminator, so the
            diagram and the program cannot drift apart without one of them being
            wrong. */}
        <div className="mt-10 rounded-xl border border-border bg-background/60 p-4 sm:mt-12 sm:p-6">
          <h3 className="font-mono text-[11px] tracking-[0.24em] text-muted-foreground">
            THE ROUND TRIP
          </h3>

          <ol className="mt-6 grid gap-5 md:grid-cols-4 md:gap-0">
            {erFlow.map((step, i) => (
              <li
                key={step.key}
                data-visible={visible}
                style={{ transitionDelay: `${240 + i * 130}ms` }}
                className="reveal group relative flex gap-4 md:block md:pr-6"
              >
                {/* The rail. Vertical on mobile, where the steps stack and the
                    column stretches to the item's height; horizontal on md,
                    where they run across. Centering uses margins, not
                    translate — the pulse keyframe owns `transform`. */}
                <div className="relative w-2.5 shrink-0 md:h-2.5 md:w-full">
                  <span
                    aria-hidden="true"
                    className={cn(
                      'absolute left-0 top-0 z-10 h-2.5 w-2.5 rounded-full transition-colors duration-500',
                      visible ? 'bg-primary' : 'bg-border-strong',
                    )}
                  />
                  {i < erFlow.length - 1 ? (
                    <span
                      aria-hidden="true"
                      style={{ transitionDelay: `${300 + i * 130}ms` }}
                      className={cn(
                        'absolute bg-border transition-transform duration-700',
                        // Mobile: down from the dot, through the row gap.
                        'left-[4.5px] top-2.5 -bottom-5 w-px origin-top',
                        visible ? 'scale-y-100' : 'scale-y-0',
                        // md: across from the dot, through the column padding.
                        'md:bottom-auto md:left-2.5 md:top-[4.5px] md:-right-6 md:h-px md:w-auto md:origin-left',
                        visible ? 'md:scale-x-100' : 'md:scale-x-0',
                      )}
                    />
                  ) : null}
                  {/* The pulse is the point of the section — state moving
                      through the round trip — so it only runs when motion is
                      welcome. */}
                  {visible && !reduced ? (
                    <span
                      aria-hidden="true"
                      style={{ animationDelay: `${i * 420}ms` }}
                      className="absolute left-0 top-0 h-2.5 w-2.5 rounded-full bg-primary animate-er-pulse"
                    />
                  ) : null}
                </div>

                <div className="min-w-0 md:mt-5">
                  <div className="flex flex-wrap items-baseline gap-x-2 font-mono text-[10px] tracking-[0.18em] text-primary">
                    <span>{step.ix}</span>
                    <span className="text-muted-foreground/60">ix {step.disc}</span>
                  </div>
                  <div className="mt-2 text-sm font-medium text-foreground">{step.label}</div>
                  <div className="mt-1 font-mono text-[11px] leading-relaxed text-muted-foreground">
                    {step.note}
                  </div>
                </div>
              </li>
            ))}
          </ol>
        </div>
      </div>
    </section>
  );
}

function Lane({
  tone,
  label,
  headline,
  body,
  marks,
  bar,
  visible,
  delay,
}: {
  tone: 'risk' | 'healthy';
  label: string;
  headline: string;
  body: string;
  marks: string[];
  bar: { fill: number; caption: string };
  visible: boolean;
  delay: number;
}) {
  return (
    <div
      data-visible={visible}
      style={{ transitionDelay: `${delay}ms` }}
      className={cn(
        'reveal rounded-xl border bg-background/60 p-5 transition-colors duration-300 sm:p-6',
        tone === 'risk' ? 'border-border hover:border-risk/40' : 'border-border hover:border-healthy/40',
      )}
    >
      <span
        className={cn(
          'font-mono text-[10px] tracking-[0.22em]',
          tone === 'risk' ? 'text-risk' : 'text-healthy',
        )}
      >
        {label}
      </span>
      <h3 className="mt-3 font-serif text-2xl leading-snug text-foreground sm:text-3xl">
        {headline}
      </h3>
      <p className="mt-3 text-[13.5px] leading-relaxed text-muted-foreground">{body}</p>

      {/* One L1 slot, drawn to scale-ish: the L1 lane gets a sliver, the ER lane
          fills it. The caption carries the real ratio. */}
      <div className="mt-5">
        <div className="h-1.5 overflow-hidden rounded-full bg-muted">
          <div
            className={cn('h-full rounded-full', tone === 'risk' ? 'bg-risk' : 'bg-healthy')}
            style={{
              width: visible ? `${bar.fill}%` : '0%',
              transition: `width 1100ms cubic-bezier(0.22,1,0.36,1) ${delay + 200}ms`,
            }}
          />
        </div>
        <p className="mt-2 font-mono text-[10.5px] text-muted-foreground">{bar.caption}</p>
      </div>

      <ul className="mt-5 space-y-2.5 border-t border-border pt-4">
        {marks.map((mark) => (
          <li key={mark} className="flex gap-2.5 font-mono text-[11.5px] leading-relaxed">
            <span
              aria-hidden="true"
              className={cn('mt-1.5 h-1 w-1 shrink-0 rounded-full', tone === 'risk' ? 'bg-risk' : 'bg-healthy')}
            />
            <span className="min-w-0 text-muted-foreground">{mark}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
