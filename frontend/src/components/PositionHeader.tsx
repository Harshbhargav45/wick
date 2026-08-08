import styles from './Console.module.css';
import { AuthorityBadge, AuthorityState } from './AuthorityBadge';

export function PositionHeader({ 
  venueLabel, 
  venueTitle, 
  authorityState 
}: { 
  venueLabel: string;
  venueTitle: string;
  authorityState: AuthorityState;
}) {
  return (
    <div className={styles.positionHeader}>
      <div>
        <span className={styles.venueTitle}>{venueTitle}</span>
        <span className={styles.venueLabel}>{venueLabel}</span>
      </div>
      <AuthorityBadge state={authorityState} />
    </div>
  );
}
