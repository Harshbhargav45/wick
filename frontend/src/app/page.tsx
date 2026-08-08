'use client';

import { useState, useEffect } from 'react';
import { ConsoleShell } from '@/components/ConsoleShell';
import { PulseStrip } from '@/components/PulseStrip';
import { PositionHeader } from '@/components/PositionHeader';
import { HealthCard } from '@/components/HealthCard';
import { PositionStats } from '@/components/PositionStats';
import { LatencyChart } from '@/components/LatencyChart';
import { LiveMeta } from '@/components/LiveMeta';
import { ActivityLog, ActivityEvent } from '@/components/ActivityLog';
import { ConfirmationSheet } from '@/components/ConfirmationSheet';
import { ToastLayer, ToastMessage } from '@/components/ToastLayer';
import styles from '@/components/Console.module.css';

const INITIAL_EVENTS: ActivityEvent[] = [
  { id: '1', type: 'healthy', message: 'Tick accepted — healthy', timeAgo: '12s ago' },
  { id: '2', type: 'healthy', message: 'Tick accepted — healthy', timeAgo: '13s ago' },
  { id: '3', type: 'action', message: 'Partial close fired — 18% (auto)', timeAgo: '2m ago' },
  { id: '4', type: 'healthy', message: 'Tick accepted — healthy', timeAgo: '2m ago' },
];

export default function Home() {
  const [authorityState, setAuthorityState] = useState<'autonomous' | 'cosigned-idle' | 'cosigned-pending'>('cosigned-idle');
  const [isSheetOpen, setIsSheetOpen] = useState(false);
  const [toasts, setToasts] = useState<ToastMessage[]>([]);

  // For hackathon demo purposes, we randomly trigger the pending state
  useEffect(() => {
    const timer = setTimeout(() => {
      setAuthorityState('cosigned-pending');
      setIsSheetOpen(true);
    }, 5000);
    return () => clearTimeout(timer);
  }, []);

  const handleConfirm = () => {
    setIsSheetOpen(false);
    setAuthorityState('cosigned-idle');
    const newToast = { id: Date.now().toString(), message: 'Confirmation sent' };
    setToasts(prev => [...prev, newToast]);
  };

  const handleDismissToast = (id: string) => {
    setToasts(prev => prev.filter(t => t.id !== id));
  };

  return (
    <div style={{ padding: 'var(--space-6) var(--space-4)', minHeight: '100vh' }}>
      <ConsoleShell>
        <PulseStrip mode="live" lastEventSeverity="healthy" />
        
        <PositionHeader 
          venueLabel="Drift · delegated" 
          venueTitle="SOL-PERP · LONG" 
          authorityState={authorityState} 
        />
        
        <HealthCard healthFactor={1.34} liquidationThreshold={1.00} triggerBuffer={1.15} />
        <PositionStats />

        <LatencyChart />
        
        <LiveMeta lastCheckTime={Date.now() - 400} slot="341,208,119" />
        
        <div className={styles.divider} />
        
        <ActivityLog events={INITIAL_EVENTS} />
      </ConsoleShell>

      <ConfirmationSheet 
        isOpen={isSheetOpen}
        onConfirm={handleConfirm}
        onDismiss={() => setIsSheetOpen(false)}
        triggerPrice="$182.50"
        positionFraction="18%"
      />
      
      <ToastLayer toasts={toasts} onDismiss={handleDismissToast} />
    </div>
  );
}
