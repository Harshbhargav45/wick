'use client';

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';

/** Reveal-on-scroll. Fires once, then stops observing. */
export function useReveal<T extends HTMLElement>(threshold = 0.1) {
  const [visible, setVisible] = useState(false);
  const ref = useRef<T>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry?.isIntersecting) {
          setVisible(true);
          observer.unobserve(el);
        }
      },
      { threshold },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [threshold]);

  return { ref, visible };
}

export function useCountUp(end: number, startAnim: boolean, durationMs = 1000, decimals = 0) {
  const [value, setValue] = useState(0);

  useEffect(() => {
    if (!startAnim) return;
    let start: number | undefined;
    let anim: number;

    const tick = (now: number) => {
      if (start === undefined) start = now;
      const progress = Math.min((now - start) / durationMs, 1);
      const easeOutQuart = 1 - Math.pow(1 - progress, 4);
      setValue(end * easeOutQuart);
      if (progress < 1) anim = requestAnimationFrame(tick);
    };
    anim = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(anim);
  }, [end, startAnim, durationMs]);

  return Number(value.toFixed(decimals));
}

const REDUCED_QUERY = '(prefers-reduced-motion: reduce)';

function subscribe(query: string) {
  return (onChange: () => void) => {
    const mq = window.matchMedia(query);
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  };
}

/**
 * Matches a media query in JS, for the cases CSS cannot reach — an SVG
 * `viewBox`, a prop, a element count. Prefer a Tailwind breakpoint class when
 * the difference is purely presentational.
 *
 * The server snapshot is `false`, so the first paint is always the
 * "unmatched" branch and the match lands on hydration.
 */
export function useMediaQuery(query: string) {
  const subscribeToQuery = useMemo(() => subscribe(query), [query]);
  const getSnapshot = useCallback(() => window.matchMedia(query).matches, [query]);
  return useSyncExternalStore(subscribeToQuery, getSnapshot, () => false);
}

export function usePrefersReducedMotion() {
  return useMediaQuery(REDUCED_QUERY);
}

function subscribeScroll(onChange: () => void) {
  window.addEventListener('scroll', onChange, { passive: true });
  return () => window.removeEventListener('scroll', onChange);
}

export function useScrolled(offset = 10) {
  const getSnapshot = useCallback(() => window.scrollY > offset, [offset]);
  return useSyncExternalStore(subscribeScroll, getSnapshot, () => false);
}
