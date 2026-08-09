import type { Metadata, Viewport } from 'next';
import { Instrument_Serif, Inter, JetBrains_Mono } from 'next/font/google';
import './globals.css';

const sans = Inter({ subsets: ['latin'], variable: '--font-sans' });
const mono = JetBrains_Mono({ subsets: ['latin'], variable: '--font-mono' });
const serif = Instrument_Serif({ weight: '400', subsets: ['latin'], variable: '--font-serif' });

export const metadata: Metadata = {
  title: {
    default: 'Wick — liquidation protection for Solana perps',
    template: '%s · Wick',
  },
  description:
    'Wick watches your leveraged perps position on Solana and acts before the liquidator — autonomous on Drift, co-signed on Jupiter.',
};

export const viewport: Viewport = {
  themeColor: '#0f0d0a',
};

const themeScript = `
(function () {
  try {
    var stored = localStorage.getItem('wick-theme');
    var dark = stored ? stored !== 'light' : true;
    document.documentElement.classList.toggle('dark', dark);
  } catch (e) {
    document.documentElement.classList.add('dark');
  }
})();
`;

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" className="dark" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: themeScript }} />
      </head>
      <body className={`${sans.variable} ${mono.variable} ${serif.variable} antialiased`}>
        {children}
      </body>
    </html>
  );
}