'use client';

import { useState } from 'react';
import Link from 'next/link';
import { ArrowUpRight, Menu, X } from 'lucide-react';
import { WickLogo } from './Logo';
import { useScrolled } from '@/hooks/useWickMotion';
import { cn } from '@/lib/utils';

const LINKS = [
  { label: 'Guard', href: '/#guard' },
  { label: 'Mechanism', href: '/#mechanism' },
  { label: 'Latency', href: '/#latency' },
];

export function Navbar() {
  const scrolled = useScrolled(16);
  const [open, setOpen] = useState(false);

  return (
    <header
      className={cn(
        'fixed inset-x-0 top-0 z-50 transition-all duration-500',
        scrolled ? 'py-2' : 'py-4',
      )}
    >
      <div className="mx-auto max-w-6xl px-4 sm:px-6">
        <nav
          aria-label="Primary"
          className={cn(
            'grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 rounded-lg px-3 py-2.5 transition-all duration-500 sm:px-4',
            scrolled
              ? 'border border-border bg-background/80 backdrop-blur-xl'
              : 'border border-transparent bg-transparent',
          )}
        >
          <div className="flex min-w-0 items-center gap-6">
            <Link href="/" className="shrink-0" aria-label="Wick home">
              <WickLogo />
            </Link>
            <ul className="hidden items-center gap-6 md:flex">
              {LINKS.map((l) => (
                <li key={l.label}>
                  <Link
                    href={l.href}
                    className="text-sm text-muted-foreground transition-colors hover:text-foreground"
                  >
                    {l.label}
                  </Link>
                </li>
              ))}
            </ul>
          </div>

          <div className="flex shrink-0 items-center gap-3">
            <span className="hidden items-center gap-2 font-mono text-[11px] tracking-wider text-muted-foreground lg:flex">
              <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-healthy animate-pulse-dot" />
              devnet · drift delegated
            </span>
            <Link
              href="/console"
              className="hidden items-center gap-1.5 rounded-md border border-border-strong px-3.5 py-1.5 text-sm font-medium text-foreground transition-all hover:border-primary hover:text-primary active:translate-y-px sm:inline-flex"
            >
              Enter console
              <ArrowUpRight className="h-3.5 w-3.5" />
            </Link>
            <button
              type="button"
              onClick={() => setOpen((v) => !v)}
              aria-expanded={open}
              aria-label={open ? 'Close menu' : 'Open menu'}
              className="inline-flex h-9 w-9 items-center justify-center rounded-md border border-border text-foreground transition-colors hover:border-border-strong md:hidden"
            >
              {open ? <X className="h-4 w-4" /> : <Menu className="h-4 w-4" />}
            </button>
          </div>
        </nav>

        {open ? (
          <div className="mt-2 overflow-hidden rounded-lg border border-border bg-background/95 backdrop-blur-xl md:hidden">
            <div className="flex items-center justify-between border-b border-border px-4 py-3">
              <span className="font-mono text-[11px] tracking-widest text-muted-foreground">
                MENU
              </span>
              <button
                type="button"
                onClick={() => setOpen(false)}
                aria-label="Close menu"
                className="text-muted-foreground transition-colors hover:text-foreground"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <ul className="p-2">
              {LINKS.map((l) => (
                <li key={l.label}>
                  <Link
                    href={l.href}
                    onClick={() => setOpen(false)}
                    className="block rounded-md px-3 py-2.5 text-sm text-muted-foreground transition-colors hover:bg-surface hover:text-foreground"
                  >
                    {l.label}
                  </Link>
                </li>
              ))}
              <li>
                <Link
                  href="/console"
                  onClick={() => setOpen(false)}
                  className="mt-1 block rounded-md bg-primary px-3 py-2.5 text-center text-sm font-medium text-primary-foreground"
                >
                  Enter console
                </Link>
              </li>
            </ul>
          </div>
        ) : null}
      </div>
    </header>
  );
}
