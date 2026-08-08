'use client';
import { useEffect, useState } from 'react';
import styles from './Console.module.css';

export function LiveMeta({ lastCheckTime, slot }: { lastCheckTime: number; slot: string }) {
  const [secondsAgo, setSecondsAgo] = useState(0);

  useEffect(() => {
    const interval = setInterval(() => {
      setSecondsAgo(Date.now() - lastCheckTime);
    }, 100); // 0.1s updates
    return () => clearInterval(interval);
  }, [lastCheckTime]);

  return (
    <div className={styles.liveMeta}>
      <div>Last check {(secondsAgo / 1000).toFixed(1)}s ago</div>
      <div>Slot <span className="mono">{slot}</span></div>
    </div>
  );
}
