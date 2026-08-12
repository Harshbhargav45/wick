# Wick — demo video script

Target: **3:00**. A 60-second cut is at the bottom.
Format: `[SCREEN]` = what the viewer sees · plain text = what you say.

Read the **Do-not-overclaim** section before recording. Three things in this
project are easy to accidentally overstate, and a judge who catches one will
discount everything else you said.

---

## Pre-flight (do this before you hit record)

- [ ] Cranker running and ticking (it is *not* running by default — it was
      stopped to save devnet SOL).
- [ ] If recording on the dev laptop: TLS relays up on `127.0.0.1:8896/8898/8899`
      (`socat` + the host-rewrite proxy). Node here cannot reach Solana RPC
      without them. On any normal machine, skip this and point straight at
      `https://api.devnet.solana.com`.
- [ ] Devnet SOL in the payer.
- [ ] Frontend running, console page loaded, wallet connected.
- [ ] A **fresh owner keypair** if you plan to re-init a guard live — an owner
      who already holds a guard cannot make another one (see §4.1a).
- [ ] Explorer tab open on the program:
      `FRtyvM3xcFhL5FbukUdzaMV7t4pePiqxPvp2ZHwptBE` (devnet).
- [ ] Do a full dry run once. The riskiest step is the live tick landing on time.

---

## The 3-minute script

### 0:00 – 0:18 · Hook

`[SCREEN] Landing page hero. Slow scroll.`

> Liquidations don't wait for you to wake up.
>
> A leveraged position can go from healthy to liquidated in under a minute. If
> you're asleep, in a meeting, or your laptop is shut — you eat the loss. The
> usual answer is "set a stop loss," but a stop is a blunt instrument: it closes
> your position when what you actually needed was fifty dollars of margin.

### 0:18 – 0:40 · What Wick is

`[SCREEN] Cut to the mechanism / stack section on the landing page.`

> Wick is an on-chain guard for perp positions. You enroll a position, set a
> policy — maintenance margin, a buffer, spending caps — and Wick watches it
> every tick.
>
> When your position breaches, Wick doesn't just close it. It picks the
> *smallest* intervention that fixes the problem: take profit if your target
> hit, top up margin if that's enough, partially close if it isn't, and escalate
> to you if nothing fits inside your caps.

### 0:40 – 1:08 · Why this needs MagicBlock

`[SCREEN] The MagicBlock / ER section. Let the animation play.`

> Here's the constraint that shapes the whole design: a guard is only useful if
> it reacts faster than the market moves against you.
>
> That's why Wick runs on MagicBlock's Ephemeral Rollup. The guard account
> delegates into the rollup, where blocks are milliseconds instead of hundreds
> of milliseconds, the guard evaluates every tick there, and state commits back
> to Solana L1. You get rollup speed for the decision and L1 settlement for the
> money.
>
> The decision logic is *on-chain* — not a bot with an API key that decides for
> you off-chain and asks you to trust the result.

### 1:08 – 2:05 · Live demo — the money shot

`[SCREEN] Split: terminal with cranker ticking on one side, console UI on the other.`

> Let's watch it happen. This is devnet, this is a real deployed program, and
> this is the live Pyth price for SOL.

`[SCREEN] Console showing a healthy guard.`

> Here's an enrolled position: forty dollars of collateral, ten units, entered at
> eighty. SOL is trading around seventy-six-fifty, so this position is
> underwater and its equity has fallen below the maintenance requirement.

`[SCREEN] Cranker tick lands. Console updates to PENDING TopUp.`

> And there it is. The guard evaluated the position on-chain and decided: top up
> thirty-three dollars and thirty-three cents.

`[SCREEN] Zoom the number: 33.330571`

> That number is worth pausing on — it's not the deficit to the liquidation
> line. It's the deficit to the liquidation line *plus the trigger buffer*.
>
> If Wick only restored you to the bare minimum, you'd be sitting exactly on the
> liquidation threshold, and the very next adverse tick re-breaches — burning
> another top-up against your daily cap, over and over. Targeting the buffer is
> what makes it stop happening.

`[SCREEN] Switch to explorer, show the guard account / recent transaction.`

> And this is on-chain state, not a dashboard rendering. Anyone can decode this
> account and check the arithmetic.

### 2:05 – 2:38 · The part that actually matters — safety rails

`[SCREEN] Policy panel showing caps, then a quick cut to the kill switch.`

> Now the boring part, which is really the whole product.
>
> Every action is capped — per action and per day, with a real accumulator, so
> the guard can't drain your margin wallet across a hundred small ticks. There's
> a global kill switch. If the price feed goes stale, the guard enters degraded
> mode and stops acting rather than acting on stale data.
>
> And authority is split in two tiers. On venues where the guard is the position
> delegate, it signs its own instruction. On Jupiter, it *cannot* act alone —
> it builds the instruction and you sign it. The guard never fakes autonomy it
> doesn't have.

### 2:38 – 3:00 · Close

`[SCREEN] Back to landing page, or a plain slide with the program ID.`

> One last thing. While building this I audited my own margin math and found
> three checks that looked enforced and did nothing — a cap comparing a
> percentage against a dollar amount, a "daily" cap with no accumulator, and a
> take-profit that fired backwards on short positions.
>
> All three are fixed, the math is property-tested, and the fix is verified
> against on-chain state — not just against a passing test suite.
>
> That's Wick. Deployed on devnet, powered by MagicBlock.

---

## Do-not-overclaim

These are the three places where the honest version and the impressive version
differ. Say the honest one — each is still a strong claim.

**1. The demo guard is `venue = none`.**
It *records* the decision; it does not dispatch to Drift or Jupiter. The
autonomous Drift path is tested against the real Velocity program, but that is
a test, not the live devnet flow. If asked "did it actually top up on Drift?" —
"the decision path is live on-chain; venue dispatch is wired and tested, and
this guard is running venue-less so the demo doesn't depend on a third-party
devnet deployment." Do not say "it topped up my Drift position."

**2. Don't claim sub-second end-to-end in the live demo.**
The ER gives millisecond blocks, and that's a real architectural claim. But the
*measured cranker tick* is ~6.1 seconds, dominated by ~3.2s of Pyth Hermes
fetch. If you say "sub-second" and someone times your video, you lose. Say the
guard evaluates in the rollup rather than waiting on L1 confirmation — true,
and enough.

**3. "Audited" means self-audited.**
No third party has reviewed this. Say "I audited my own math and found three
bugs" — which is more credible than an unsourced "audited," and the bugs are
genuinely interesting.

Also: the caps, kill switch, and degraded mode are real and tested. The
authority split is real. The program address is stable across upgrades. Those
you can state flatly.

---

## The 60-second cut

> Liquidations don't wait for you to wake up. A leveraged position can go from
> healthy to liquidated in under a minute.
>
> Wick is an on-chain guard for perp positions. It watches your position every
> tick, and when it breaches, it picks the smallest fix that works — top up
> margin, partially close, or escalate to you if nothing fits your caps.
>
> `[Live: cranker tick lands, console shows PENDING TopUp 33.330571]`
>
> There. The guard decided on-chain to add thirty-three dollars — enough to
> clear the breach *and* the safety buffer, so the next tick doesn't
> immediately re-breach.
>
> It runs on MagicBlock's Ephemeral Rollup, so the decision happens at rollup
> speed and settles on Solana. Every action is capped daily, there's a kill
> switch, and on co-signed venues the guard can only *build* the instruction —
> you sign it.
>
> Deployed on devnet. Powered by MagicBlock.

---

## If the live demo breaks

Have a recorded fallback clip of a successful tick. The failure mode most likely
to bite you is a tick not landing in time (devnet RPC throttling, or the Hermes
fetch running long). If it stalls on camera, say "devnet RPC is rate-limiting —
here's the same flow from a minute ago" and cut to the clip. That reads as
prepared, not broken.
