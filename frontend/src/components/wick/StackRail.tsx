import { stackCards } from '@/lib/wick-data';
import { Marquee } from './Marquee';
import { Reveal } from './Reveal';

/**
 * The moving stack rail. Every card names a real enforcement point in the
 * program, so the row reads as a summary of what runs on a tick rather than a
 * logo carousel.
 */
export function StackRail() {
  return (
    <section aria-labelledby="stack-rail-heading" className="border-b border-border/70 py-14">
      <Reveal className="mx-auto max-w-6xl px-4 sm:px-6">
        <h2
          id="stack-rail-heading"
          className="font-mono text-[11px] tracking-[0.24em] text-muted-foreground/70"
        >
          WHAT RUNS ON A TICK
        </h2>
      </Reveal>

      <Marquee
        label="What runs on every tick"
        durationS={52}
        className="mt-7"
      >
        {stackCards.map((card) => (
          <article
            key={card.key}
            tabIndex={0}
            className="mr-4 flex w-[264px] shrink-0 flex-col rounded-xl border border-border bg-surface/40 p-5 transition-colors duration-300 hover:border-border-strong hover:bg-surface/70 focus-visible:border-primary"
          >
            <span className="font-mono text-[10px] tracking-[0.2em] text-primary">{card.tag}</span>
            <h3 className="mt-3 font-serif text-xl text-foreground">{card.title}</h3>
            <p className="mt-2 text-[13px] leading-relaxed text-muted-foreground">{card.body}</p>
          </article>
        ))}
      </Marquee>
    </section>
  );
}
