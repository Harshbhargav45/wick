import type { Metadata } from 'next';
import './globals.css';

export const metadata: Metadata = {
  title: 'Wick Sentinel',
  description: 'Wick watches your position and acts before you have to.',
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
