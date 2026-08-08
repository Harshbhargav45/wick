import styles from './Console.module.css';

export function IgnitionMark({ className }: { className?: string }) {
  return (
    <svg 
      viewBox="0 0 64 48" 
      className={className} 
      style={{ width: '32px', height: '24px', overflow: 'visible' }}
    >
      <path 
        d="M 4,8 L 16,40 L 26,16 L 36,40 L 46,16 L 58,8" 
        fill="none" 
        stroke="var(--text-primary)" 
        strokeWidth="4" 
        strokeLinecap="square" 
        strokeLinejoin="miter" 
      />
      <circle cx="58" cy="8" r="3" fill="var(--accent-ember)" />
    </svg>
  );
}
