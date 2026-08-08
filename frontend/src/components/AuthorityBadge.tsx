import styles from './Console.module.css';

export type AuthorityState = 'autonomous' | 'cosigned-idle' | 'cosigned-pending';

export function AuthorityBadge({ state }: { state: AuthorityState }) {
  let badgeClass = styles.authorityBadge;
  let label = '';
  
  if (state === 'autonomous') {
    badgeClass += ` ${styles.authAutonomous}`;
    label = 'AUTONOMOUS';
  } else if (state === 'cosigned-idle') {
    badgeClass += ` ${styles.authCosignedIdle}`;
    label = 'CO-SIGNED';
  } else if (state === 'cosigned-pending') {
    badgeClass += ` ${styles.authCosignedPending}`;
    label = 'AWAITING YOUR CONFIRMATION';
  }

  return (
    <div className={badgeClass}>
      <div className={styles.badgeDot} />
      {label}
    </div>
  );
}
