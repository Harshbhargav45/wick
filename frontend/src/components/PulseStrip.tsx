import styles from './Console.module.css';

export function PulseStrip({ mode, lastEventSeverity }: { mode: 'boot' | 'live', tickTimestamps?: number[], lastEventSeverity?: 'healthy' | 'warn' | 'danger' | null }) {
  const isWarn = lastEventSeverity === 'warn';
  const isDanger = lastEventSeverity === 'danger';
  
  let dotClass = styles.pulseDot;
  if (isWarn) dotClass += ` ${styles.pulseDotWarn}`;
  if (isDanger) dotClass += ` ${styles.pulseDotDanger}`;
  
  let lineClass = styles.pulseLine;
  if (mode === 'live') {
    lineClass += ` ${styles.pulseLineLive}`;
  }

  return (
    <div className={styles.pulseStripWrapper}>
      <div className={lineClass} />
      {mode === 'live' && (
        <div className={dotClass} />
      )}
    </div>
  );
}
