'use client';

import { useEffect, useRef, useState } from 'react';
import { Check, Copy } from 'lucide-react';
import { cn } from '@/lib/utils';

/**
 * Copy-to-clipboard with a short-lived confirmation.
 *
 * A refused clipboard is not an error worth a banner — the value is still on
 * screen and selectable — so failure just leaves the button unconfirmed.
 */
export function CopyButton({
  value,
  label = 'Copy',
  copiedLabel = 'Copied',
  showLabel = false,
  className,
}: {
  value: string;
  label?: string;
  copiedLabel?: string;
  showLabel?: boolean;
  className?: string;
}) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => setCopied(false), 2000);
    } catch {
      setCopied(false);
    }
  };

  const Icon = copied ? Check : Copy;

  return (
    <button
      type="button"
      onClick={copy}
      aria-label={copied ? copiedLabel : label}
      title={copied ? copiedLabel : label}
      className={cn(
        'inline-flex items-center gap-1.5 rounded border border-border px-1.5 py-1 font-mono text-[10.5px] text-muted-foreground transition-colors hover:border-border-strong hover:text-foreground',
        className,
      )}
    >
      <Icon className="h-3 w-3" aria-hidden="true" />
      {showLabel ? <span>{copied ? copiedLabel : label}</span> : null}
    </button>
  );
}
