'use client';

import type { ElementType, ReactNode } from 'react';
import { useReveal } from '@/hooks/useWickMotion';
import { cn } from '@/lib/utils';

interface RevealProps {
  children: ReactNode;
  className?: string;
  delay?: number;
  as?: ElementType;
}

export function Reveal({ children, className, delay = 0, as: Tag = 'div' }: RevealProps) {
  const { ref, visible } = useReveal<HTMLDivElement>();
  return (
    <Tag
      ref={ref}
      data-visible={visible}
      style={{ transitionDelay: `${delay}ms` }}
      className={cn('reveal', className)}
    >
      {children}
    </Tag>
  );
}
