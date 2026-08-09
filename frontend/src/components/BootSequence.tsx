'use client';
import { useEffect, useState } from 'react';
import styles from './Console.module.css';

export function BootSequence({ onComplete }: { onComplete: () => void }) {
  const [isBooting, setIsBooting] = useState(true);

  useEffect(() => {
    // Check reduced motion
    const prefersReduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

    const timer = setTimeout(() => {
      setIsBooting(false);
      onComplete();
    }, prefersReduced ? 0 : 1800); // 1.8s boot sequence per spec

    return () => clearTimeout(timer);
  }, [onComplete]);

  if (!isBooting) return null;

  return (
    <div className={styles.bootOverlay}>
      <div className={styles.bootPulseStripWrapper}>
         <div className={styles.bootPulseLine} />
         <div className={styles.bootPulseDot} />
      </div>
    </div>
  );
}
