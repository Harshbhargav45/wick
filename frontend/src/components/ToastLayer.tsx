'use client';
import { useEffect, useState } from 'react';
import styles from './Console.module.css';

export type ToastMessage = {
  id: string;
  message: string;
  isError?: boolean;
};

export function ToastLayer({ toasts, onDismiss }: { toasts: ToastMessage[]; onDismiss: (id: string) => void }) {
  return (
    <div className={styles.toastLayer}>
      {toasts.map(t => (
        <Toast key={t.id} toast={t} onDismiss={() => onDismiss(t.id)} />
      ))}
    </div>
  );
}

function Toast({ toast, onDismiss }: { toast: ToastMessage; onDismiss: () => void }) {
  useEffect(() => {
    if (!toast.isError) {
      const timer = setTimeout(onDismiss, 4000);
      return () => clearTimeout(timer);
    }
  }, [toast, onDismiss]);

  return (
    <div className={styles.toast} onClick={toast.isError ? onDismiss : undefined} style={{ cursor: toast.isError ? 'pointer' : 'default' }}>
      {toast.message}
    </div>
  );
}
