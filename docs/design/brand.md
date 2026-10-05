# Boba Job Find — Brand

> **Status:** final draft · **Owner:** ux · **Date:** 2026-10-04
> Companion docs: [`DESIGN.md`](./DESIGN.md) (visual system), [`voice-and-tone.md`](./voice-and-tone.md) (words),
> [`EXPERIENCE.md`](./EXPERIENCE.md) (behavior). Brand assets in [`assets/`](./assets/).

---

## 1. Name check — *Boba Job Find*

**Verdict:** keep it. Clear, warm, memorable, ownable. It says **what it is** (job finding) and
**how it feels** (boba — playful, calm, a treat).

| Aspect | Read |
|---|---|
| Distinctiveness | High — "boba" is uncommon in the job-tools space (a category full of `Job*`, `*Hunt`, `*Apply`). |
| Meaning | A boba drink is *assembled* from chosen pearls — a fitting metaphor for a curated shortlist. |
| Tone | Friendly and low-stakes, exactly the antidote to job-search anxiety. |
| Pronunciation | `BO-ba JOB find` — unambiguous in EN; in ES it reads naturally (`boba`, `trabajo`, `encontrar`). |
| Colloquialism | "Job find" is informal. We keep it: it is honest and warm, not corporate. The full name is the brand; "Boba" is the friendly short form. |
| Search/SEO | "boba" collides with bubble tea; always pair with **Job Find** in titles, repos, and metadata. |

**Optional refinements (UX proposal — the locked name wins if rejected):**
1. **Wordmark casing** — keep legal/display name `Boba Job Find`; style the wordmark as
   **`boba`** (lowercase, display face) over **`Job Find`**. Keeps the taste, stays searchable.
2. **Short name** — **"Boba"** in-app and in the tray tooltip. The mascot is *Tapi* (see §5);
   Boba is the product, Tapi is the character.
3. **Rejected refinement:** `Boba Job Finder`. Grammatical, but "Job Find" is more in the brand's
   loose, friendly register and echoes the upstream ("ai-job-search"). Flagged, not adopted.

### Taglines

| Role | EN | ES |
|---|---|---|
| **Primary (chosen)** | **Steep your search. Sip the good offers.** | **Deja reposar la búsqueda. Saborea las buenas ofertas.** |
| Short / app + tray | **Good things steep.** | **Lo bueno se toma su tiempo.** |
| Utility / repo + docs | **A local-first job search that finds and filters, so you only apply where it counts.** | **Una búsqueda local que filtra por ti: postúlate solo donde vale la pena.** |
| The honest-coach line | **We do the steeping. You pick the pearls.** | **Nosotros preparamos el té. Tú eliges las perlas.** |

Tagline usage rule: the primary tagline appears **once** per surface (landing/overview/welcome).
The utility line is for metadata and READMEs. The honest-coach line carries the differentiator:
the app does the tedious work, the human keeps the decision.

---

## 2. Brand concept & metaphor

The product is a **boba tea shop for job hunting**. It turns a stressful, opaque chore into a warm,
slow-brewed ritual: something is *prepared for you daily*, and you choose what to drink.

| Boba element | Product meaning |
|---|---|
| **Loose tea leaves** | Raw job postings — unordered, bitter on their own. |
| **Steeping** | The daily search job. Good results need the right temperature and a little time — it runs in the background while you live. |
| **The cup** | Your pipeline: today's shortlist → tracker. Clear, held together, yours. |
| **Tapioca pearls** | **Opportunities.** Each one is a match worth bottling. The good ones are plump and glossy. |
| **Pearl gloss / size** | Offer quality — how appetizing this opportunity is. |
| **Sweetness level** | Your preferences and scoring weights: "50% sugar", "more fit, less flash". Tunable, never hidden. |
| **Flavors** | The two score axes: **matcha = Fit** (can I get it?), **taro = Quality** (do I want it?). |
| **The straw** | The path from a chosen offer to a sent application — one clean, deliberate motion. |
| **Milk tea** | The calm layer over a stressful task. Warmth, not hype. |

**Emotional promise:** *"This is handled. You can breathe. Here's what's worth your energy today."*

**What the metaphor never means:** no gambling, no slot-machine rewards, no "spin to win".
Job hunting is high-stakes; the boba is a soothing ritual, not a casino.

---

## 3. Brand personality

| Attribute | We are | We are not |
|---|---|---|
| **Warm** | A kind coach who knows the market | A chirpy hype bot |
| **Honest** | We say when a role is a stretch, and why | We hide bad news behind positive color |
| **Calm** | Slow, uncluttered, one clear next step | Urgent, noisy, fear-of-missing-out |
| **Capable** | It did the hard scanning and scoring for you | A toy that just looks cute |
| **Yours** | Local, private, no accounts, you decide | A cloud that watches you |

**Voice in one line:** *a warm, honest job-search coach who happens to make great boba.*
Full words and microcopy live in [`voice-and-tone.md`](./voice-and-tone.md).

---

## 4. Logo concept — **"The Pearl Lens"**

A magnifying glass (search) whose **lens is a boba cup**: the glass holds three glossy tapioca pearls
(opportunities already found), and a **straw** rises from the top-right. It fuses the two ideas of the
product — *finding* and *choosing* — into one mark.

- **Construction:** 2px-equivalent rounded strokes, geometric, generous internal padding,
  single-weight outline. Pearls as solid circles inside the lens.
- **Clear space:** ≥ 0.5 × mark height on all sides.
- **Minimum size:** 20px (icon) / 24px (favicon, with the straw simplified to a nub below 24px).
- **Colorways:** full-color (primary caramel stroke + matcha/taro/caramel pearls), mono-dark
  (espresso), mono-light (cream on dark). The mark must read at 16px in one color.
- **Do not:** add gradients, drop shadows, or more than three pearls; do not rotate; do not place
  the wordmark inside the lens.

Assets:
- [`assets/logo-mark.svg`](./assets/logo-mark.svg) — color mark
- [`assets/logo-mark-mono.svg`](./assets/logo-mark-mono.svg) — one-color mark
- [`assets/wordmark.svg`](./assets/wordmark.svg) — lockup (TBD: wordmark SVG built at handoff; see §6)

---

## 5. Mascot — **Tapi**

**Tapi** is a single plump tapioca pearl: a warm caramel-brown sphere with a soft glossy highlight,
a tiny tea-leaf sprout on top, two dot eyes, and a small smile. Minimal, geometric, expressive through
**eyes + pose only** — so one SVG can carry a whole emotional range.

**Why a pearl, not a cup:** pearls are the *opportunities*; Tapi is the friend who points them out.
The user is the one holding the cup.

| Tapi state | Pose / face | Used in |
|---|---|---|
| Idle | Neutral blink, tiny smile | Sidebar footer, page chrome |
| Peeking | Half behind a card edge | Empty states |
| Cheering | Leaf sprout up, eyes happy arcs | New matches, application sent |
| Thinking | Tilted, one eye squint | Daily run in progress |
| Sleepy | Closed eyes, "z" | Background/tray, overnight run |
| Pointing | Small arm out | Onboarding, tooltips |
| Umbrella | Tiny umbrella over Tapi | Low-fit / rejection comfort |
| Determined | Flat mouth, focused | Auto-apply run |

**Rules:** Tapi appears at *emotional beats* only (first run, empty, success, comfort) — never on
data-dense screens where it competes with content. Never speaks in baby-talk; Tapi's lines are the
coach's lines. Never used as an icon substitute. Max ~120px in-product, ~64px in notifications.

Assets: [`assets/mascot-tapi.svg`](./assets/mascot-tapi.svg) (idle, the base state).

---

## 6. Brand application map

| Surface | Beads of brand |
|---|---|
| App icon / tray | Pearl Lens mark on cream (light) / espresso (dark). |
| Sidebar | Logo + "Boba" wordmark; Tapi idle in the footer. |
| Onboarding | Tapi "pointing" guides each step; welcome uses primary tagline. |
| Empty states | Tapi "peeking" + one clear CTA. |
| New matches / digest | Tapi "cheering", pearls metaphor in copy ("3 new pearls today"). |
| Low fit / rejection | Tapi "umbrella" + honest, kind copy. |
| Errors | No mascot — calm inline message, plain language. |
| Docs / README / overview page | Primary tagline once, utility line in metadata, palette + type specimens from `DESIGN.md`. |

---

## 7. Brand do's and don'ts

**Do**
- Lead with **warmth + honesty**: warmth in the surface, honesty in the data.
- Keep **one obvious next step** per screen.
- Show **both** scores (Fit and Quality), with the *why*.
- Use the **mascot at emotional beats** and nowhere else.
- Keep supply of color: cream/espresso surfaces, caramel primary, matcha/taro accents.
- Stay **local-first and private** in language ("on your machine", "never leaves your computer").

**Don't**
- Don't infantilize: no baby-talk, no cutesy misspellings, no "uwu".
- Don't gamify applications: no streaks, no confetti storms, no pressure counters.
- Don't hide a bad fit behind a friendly color — a stretch is labeled a stretch.
- Don't use emoji as icons (Lucide + custom SVGs only).
- Don't put brown text on brown, or rely on hue alone for any score (see `DESIGN.md` §score-system).
- Don't promise outcomes ("you'll get this job") — promise **effort made visible**.
- Don't put Tapi in loading states or error messages.
