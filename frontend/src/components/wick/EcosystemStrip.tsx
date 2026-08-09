import { ecosystem } from '@/lib/wick-data';
import { Reveal } from './Reveal';

export function EcosystemStrip() {
  return (
    <section className="border-y border-border/70 bg-surface/30">
      <Reveal className="mx-auto flex max-w-6xl flex-wrap items-center justify-center gap-x-10 gap-y-4 px-4 py-8 sm:px-6">
        <span className="font-mono text-[11px] tracking-[0.24em] text-muted-foreground/70">
          WITH
        </span>
        {ecosystem.map((name) => (
          <span
            key={name}
            className="font-mono text-sm tracking-[0.12em] text-muted-foreground transition-all duration-300 hover:-translate-y-0.5 hover:text-foreground"
          >
            {name}
          </span>
        ))}
      </Reveal>
    </section>
  );
}
