'use client';

import { Moon, Sun } from 'lucide-react';
import { cn } from '@/lib/utils';

/**
 * Flips the `dark` class the inline boot script in `layout.tsx` already reads,
 * and persists the choice under the same key.
 *
 * The icons are chosen in CSS (`dark:block` / `dark:hidden`) rather than from
 * React state, so the first paint never disagrees with the class on `<html>` and
 * there is no hydration mismatch to suppress.
 */
export function ThemeToggle({ className }: { className?: string }) {
  const toggle = () => {
    const root = document.documentElement;
    const dark = root.classList.toggle('dark');
    try {
      localStorage.setItem('wick-theme', dark ? 'dark' : 'light');
    } catch {
      // Storage can be disabled; the class change still applies for this visit.
    }
    document
      .querySelector('meta[name="theme-color"]')
      ?.setAttribute('content', dark ? '#0f0d0a' : '#faf7f2');
  };

  return (
    <button
      type="button"
      onClick={toggle}
      aria-label="Toggle color theme"
      title="Toggle color theme"
      className={cn(
        'grid h-9 w-9 place-items-center rounded-md border border-border text-muted-foreground transition-colors hover:border-border-strong hover:text-foreground',
        className,
      )}
    >
      <Sun className="hidden h-4 w-4 dark:block" aria-hidden="true" />
      <Moon className="h-4 w-4 dark:hidden" aria-hidden="true" />
    </button>
  );
}
