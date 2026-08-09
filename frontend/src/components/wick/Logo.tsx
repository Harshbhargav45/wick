import { cn } from '@/lib/utils';

export function WickMark({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 32 20"
      aria-hidden="true"
      className={cn('h-4 w-6 text-primary', className)}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="square"
    >
      <path d="M1 2 L8 18 L16 6 L24 18 L31 2" />
    </svg>
  );
}

export function WickLogo({ className }: { className?: string }) {
  return (
    <span className={cn('flex items-center gap-2.5', className)}>
      <WickMark />
      <span className="font-mono text-sm font-semibold tracking-[0.28em] text-foreground">
        WICK
      </span>
    </span>
  );
}
