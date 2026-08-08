import { ReactNode } from 'react';
import styles from './Console.module.css';

export function ConsoleShell({ children }: { children: ReactNode }) {
  return (
    <div className={styles.shell}>
      {children}
    </div>
  );
}
