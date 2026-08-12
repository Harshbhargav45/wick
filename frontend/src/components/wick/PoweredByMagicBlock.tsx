import { magicblock } from '@/lib/wick-data';
import { cn } from '@/lib/utils';

/**
 * The MagicBlock mark — a block with a cut corner, matching the angular
 * treatment of the Wick mark rather than importing a raster logo.
 */
function MagicBlockMark({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 16 16"
      aria-hidden="true"
      className={cn('shrink-0', className)}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinejoin="miter"
    >
      <path d="M2 2 H14 V10 L10 14 H2 Z" />
      <path d="M6 8 H10" strokeWidth="1.2" />
    </svg>
  );
}

type Variant = 'badge' | 'inline' | 'feature';

const CHROME: Record<Variant, string> = {
  // Footer / hero: reads as a real credit, not a caption.
  feature:
    'gap-3 rounded-lg border border-border bg-surface/60 px-4 py-3 text-sm text-muted-foreground hover:border-primary hover:bg-surface/80 hover:text-foreground',
  badge:
    'gap-2.5 rounded-md border border-border bg-surface/60 px-3 py-2 text-[13px] text-muted-foreground hover:border-primary hover:text-foreground',
  inline: 'gap-2 text-[12.5px] text-muted-foreground hover:text-foreground',
};

const MARK_SIZE: Record<Variant, string> = {
  feature: 'h-5 w-5',
  badge: 'h-4 w-4',
  inline: 'h-3.5 w-3.5',
};

/**
 * Attribution for the Ephemeral Rollup the guard delegates into (§8.6). Links
 * out to MagicBlock; `variant` only changes the chrome, not the wording.
 */
export function PoweredByMagicBlock({
  variant = 'badge',
  className,
}: {
  variant?: Variant;
  className?: string;
}) {
  return (
    <a
      href={magicblock.href}
      target="_blank"
      rel="noreferrer"
      className={cn(
        'group inline-flex items-center font-mono transition-colors',
        CHROME[variant],
        className,
      )}
    >
      <MagicBlockMark
        className={cn(
          'text-primary transition-transform duration-300 group-hover:scale-110',
          MARK_SIZE[variant],
        )}
      />
      <span className="tracking-[0.1em]">
        Powered by{' '}
        <span className={cn('text-foreground', variant === 'feature' && 'font-semibold')}>
          {magicblock.name}
        </span>
      </span>
      {variant === 'feature' ? (
        <span className="ml-1 border-l border-border pl-3 text-[11px] tracking-[0.16em] text-muted-foreground/70">
          {magicblock.tagline.toUpperCase()}
        </span>
      ) : null}
    </a>
  );
}
