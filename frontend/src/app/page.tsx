import { Navbar } from '@/components/wick/Navbar';
import { Hero } from '@/components/wick/Hero';
import { EcosystemStrip } from '@/components/wick/EcosystemStrip';
import { GuardCards } from '@/components/wick/GuardCards';
import { LatencySection } from '@/components/wick/LatencySection';
import { Mechanism } from '@/components/wick/Mechanism';
import { Footer } from '@/components/wick/Footer';

export default function LandingPage() {
  return (
    <div className="min-h-screen bg-background text-foreground">
      <Navbar />
      <main>
        <Hero />
        <EcosystemStrip />
        <GuardCards />
        <LatencySection />
        <Mechanism />
      </main>
      <Footer />
    </div>
  );
}
