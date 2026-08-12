import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';

interface MarqueeProps {
  children: ReactNode;
  /** Seconds for one full pass. Longer rails need longer durations to read. */
  durationS?: number;
  reverse?: boolean;
  className?: string;
  label: string;
}

/**
 * A seamless horizontal rail.
 *
 * The children are rendered twice and the track translates by exactly -50%, so
 * the animation ends on a frame identical to its start. The duplicate is
 * `aria-hidden` — a screen reader walks the real list once, and the whole rail
 * is a labelled group so it is not read as a wall of loose links.
 */
export function Marquee({
  children,
  durationS = 40,
  reverse = false,
  className,
  label,
}: MarqueeProps) {
  return (
    <div className={cn('marquee', className)} role="group" aria-label={label}>
      <div
        className="marquee-track"
        data-direction={reverse ? 'reverse' : undefined}
        style={{ '--marquee-duration': `${durationS}s` } as React.CSSProperties}
      >
        <div className="flex shrink-0 items-stretch">{children}</div>
        <div aria-hidden="true" className="flex shrink-0 items-stretch">
          {children}
        </div>
      </div>
    </div>
  );
}
