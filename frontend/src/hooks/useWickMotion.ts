'use client';

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react';

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

function subscribeReduced(onChange: () => void) {
  const mq = window.matchMedia(REDUCED_QUERY);
  mq.addEventListener('change', onChange);
  return () => mq.removeEventListener('change', onChange);
}

export function usePrefersReducedMotion() {
  return useSyncExternalStore(
    subscribeReduced,
    () => window.matchMedia(REDUCED_QUERY).matches,
    () => false,
  );
}

function subscribeScroll(onChange: () => void) {
  window.addEventListener('scroll', onChange, { passive: true });
  return () => window.removeEventListener('scroll', onChange);
}

export function useScrolled(offset = 10) {
  const getSnapshot = useCallback(() => window.scrollY > offset, [offset]);
  return useSyncExternalStore(subscribeScroll, getSnapshot, () => false);
}
