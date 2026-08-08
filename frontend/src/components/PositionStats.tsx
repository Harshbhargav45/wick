import styles from './Console.module.css';

export function PositionStats() {
  return (
    <div className={styles.positionStats}>
      <div className={styles.statRow}>
        <span className={styles.statLabel}>Collateral</span>
        <span className={styles.statValue}>$2,140.00</span>
      </div>
      <div className={styles.statRow}>
        <span className={styles.statLabel}>Entry</span>
        <span className={styles.statValue}>$184.20</span>
      </div>
      <div className={styles.statRow}>
        <span className={styles.statLabel}>Size</span>
        <span className={styles.statValue}>-12.4 SOL</span>
      </div>
      <div className={styles.statRow}>
        <span className={styles.statLabel}>Current</span>
        <span className={styles.statValue}>$181.90</span>
      </div>
      <div className={styles.statRow}>
        <span className={styles.statLabel}>Unrealized PnL</span>
        <span className={`${styles.statValue} ${styles.statValueDanger}`}>
          -$28.52 <span className={styles.statSecondary}>(-1.3%)</span>
        </span>
      </div>
    </div>
  );
}
