/**
 * First-paint placeholder for `/console`.
 *
 * Shaped like the real two-column grid rather than a spinner, so the panels do
 * not jump when the first snapshot lands. `aria-busy` plus the sr-only line tell
 * a screen reader the state is still loading; the blocks themselves are
 * presentation only.
 */
export function ConsoleSkeleton() {
  return (
    <div aria-busy="true" aria-live="polite">
      <span className="sr-only">Loading guard state</span>

      <div className="flex flex-wrap items-end justify-between gap-4">
        <div className="space-y-2">
          <div className="h-8 w-44 animate-pulse rounded bg-muted" />
          <div className="h-3 w-64 animate-pulse rounded bg-muted" />
        </div>
        <div className="h-7 w-44 animate-pulse rounded bg-muted" />
      </div>

      <div className="mt-6 grid gap-4 lg:grid-cols-[1fr_320px]">
        <div className="space-y-4">
          <Block className="h-64" />
          <Block className="h-44" />
          <Block className="h-56" />
          <Block className="h-52" />
        </div>
        <div className="space-y-4">
          <Block className="h-56" />
          <Block className="h-40" />
          <Block className="h-64" />
          <Block className="h-40" />
          <Block className="h-56" />
        </div>
      </div>
    </div>
  );
}

function Block({ className }: { className: string }) {
  return (
    <div
      aria-hidden="true"
      className={`animate-pulse rounded-xl border border-border bg-surface/40 ${className}`}
    />
  );
}
