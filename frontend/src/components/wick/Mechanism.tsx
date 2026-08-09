'use client';

import { mechanism } from '@/lib/wick-data';
import { useReveal } from '@/hooks/useWickMotion';
import { Reveal } from './Reveal';

export function Mechanism() {
  const { ref, visible } = useReveal<HTMLDivElement>(0.25);

  return (
    <section id="mechanism" className="border-t border-border/70 bg-surface/20 py-24 sm:py-32">
      <div className="mx-auto max-w-6xl px-4 sm:px-6">
        <Reveal className="max-w-2xl">
          <span className="font-mono text-[11px] tracking-[0.24em] text-primary">
            03 — MECHANISM
          </span>
          <h2 className="mt-4 font-serif text-4xl leading-tight text-foreground sm:text-5xl">
            One tick, eight stages
          </h2>
          <p className="mt-4 text-muted-foreground">
            The critical path inside a single <span className="font-mono text-sm">OnPriceTick</span>{' '}
            instruction. The order is load-bearing: price before health, health before nonce, caps
            before dispatch. Any stage can reject, and a rejected tick mutates nothing.
          </p>
        </Reveal>

        <div ref={ref} className="mt-14">
          <ol className="relative grid gap-px overflow-hidden rounded-xl border border-border bg-border md:grid-cols-4">
            {mechanism.map((step, i) => (
              <li
                key={step.key}
                data-visible={visible}
                style={{ transitionDelay: `${i * 70}ms` }}
                className="reveal group relative bg-background p-5 transition-colors duration-300 hover:bg-surface"
              >
                <div className="flex items-center justify-between">
                  <span className="font-mono text-[10px] tracking-[0.2em] text-muted-foreground/70">
                    {String(i + 1).padStart(2, '0')}
                  </span>
                  <span
                    aria-hidden="true"
                    className="h-1.5 w-1.5 rounded-full bg-border-strong transition-colors duration-300 group-hover:bg-primary"
                  />
                </div>
                <div className="mt-4 text-sm font-medium text-foreground">{step.label}</div>
                <div className="mt-1 font-mono text-[11px] text-muted-foreground">{step.note}</div>
                <span
                  aria-hidden="true"
                  className="absolute inset-x-0 bottom-0 h-px origin-left scale-x-0 bg-primary transition-transform duration-500 group-hover:scale-x-100"
                />
              </li>
            ))}
          </ol>

          <div className="relative mt-6 h-px overflow-hidden bg-border" aria-hidden="true">
            {visible ? (
              <div className="absolute inset-y-0 w-24 bg-gradient-to-r from-transparent via-primary to-transparent animate-sweep" />
            ) : null}
          </div>
          <p className="mt-4 text-center font-mono text-[11px] tracking-[0.2em] text-muted-foreground">
            WATCH → DETECT → ACT → CONFIRM
          </p>
        </div>
      </div>
    </section>
  );
}
