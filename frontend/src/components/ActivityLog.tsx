import styles from './Console.module.css';

export type ActivityEvent = {
  id: string;
  type: 'healthy' | 'action' | 'warn';
  message: string;
  timeAgo: string;
};

export function ActivityLog({ events }: { events: ActivityEvent[] }) {
  return (
    <div className={styles.activityLog}>
      <div className={styles.activityLogTitle}>ACTIVITY</div>
      {events.map((event) => (
        <div key={event.id} className={styles.activityRow}>
          <div className={`${styles.activityDot} ${styles[event.type]}`} />
          <div className={`${styles.activityContent} ${styles[event.type]}`}>
            {event.message}
          </div>
          <div className={styles.activityTime}>{event.timeAgo}</div>
        </div>
      ))}
    </div>
  );
}
