import styles from './Console.module.css';

export function HealthCard({
  healthFactor,
  liquidationThreshold = 1.00,
  triggerBuffer = 1.15
}: {
  healthFactor: number;
  liquidationThreshold?: number;
  triggerBuffer?: number;
}) {
  const isDanger = healthFactor <= liquidationThreshold;
  const isWarn = healthFactor <= triggerBuffer && !isDanger;
  
  let valClass = styles.healthValue;
  if (isDanger) valClass += ` ${styles.healthValueDanger}`;
  else if (isWarn) valClass += ` ${styles.healthValueWarn}`;
  
  let fillClass = styles.healthBarFill;
  if (isDanger) fillClass += ` ${styles.healthBarFillDanger}`;
  else if (isWarn) fillClass += ` ${styles.healthBarFillWarn}`;

  // Max value for bar is 2.0 to show some headroom
  const maxFactor = 2.0;
  const fillWidth = Math.min(100, Math.max(0, (healthFactor / maxFactor) * 100));
  
  const liqPos = (liquidationThreshold / maxFactor) * 100;
  const trigPos = (triggerBuffer / maxFactor) * 100;

  return (
    <div className={styles.healthCard}>
      <div className={styles.healthLabel}>HEALTH FACTOR</div>
      <div className={valClass}>{healthFactor.toFixed(2)}</div>
      
      <div className={styles.healthBarTrack}>
        <div className={fillClass} style={{ width: `${fillWidth}%` }} />
        
        {/* Liquidation Threshold */}
        <div className={`${styles.healthThreshold} ${styles.healthThresholdLiq}`} style={{ left: `${liqPos}%` }} />
        <div className={styles.healthThresholdLabel} style={{ left: `${liqPos}%` }}>liq. at {liquidationThreshold.toFixed(2)}</div>
        
        {/* Trigger Buffer */}
        <div className={styles.healthThreshold} style={{ left: `${trigPos}%` }} />
        <div className={styles.healthThresholdLabel} style={{ left: `${trigPos}%` }}>trigger at {triggerBuffer.toFixed(2)}</div>
      </div>
    </div>
  );
}
