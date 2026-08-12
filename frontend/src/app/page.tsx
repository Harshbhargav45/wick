import { Navbar } from '@/components/wick/Navbar';
import { Hero } from '@/components/wick/Hero';
import { EcosystemStrip } from '@/components/wick/EcosystemStrip';
import { StackRail } from '@/components/wick/StackRail';
import { GuardCards } from '@/components/wick/GuardCards';
import { LatencySection } from '@/components/wick/LatencySection';
import { ErSection } from '@/components/wick/ErSection';
import { Mechanism } from '@/components/wick/Mechanism';
import { Footer } from '@/components/wick/Footer';

export default function LandingPage() {
  return (
    <div className="min-h-screen bg-background text-foreground">
      <Navbar />
      <main>
        <Hero />
        <EcosystemStrip />
        <StackRail />
        <GuardCards />
        <LatencySection />
        {/* Speed first, then why the guard can be that fast, then the per-tick
            detail — the ER section answers the question the latency numbers
            raise. */}
        <ErSection />
        <Mechanism />
      </main>
      <Footer />
    </div>
  );
}
