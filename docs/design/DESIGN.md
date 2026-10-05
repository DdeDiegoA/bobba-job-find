---
version: alpha
name: Boba Job Find
description: A warm, playful, local-first desktop job-search system. Soft rounded shapes, warm tea/tapioca palette, calm against job-search stress. Light + dark, EN + ES.
status: final draft
updated: 2026-10-04
sources:
  - docs/design/brand.md
  - docs/design/voice-and-tone.md
  - docs/design/EXPERIENCE.md
colors:
  # primitives (excerpt; full ramps in §2)
  caramel-600: "#9A5418"
  caramel-700: "#7E4312"
  cream-50: "#FFFBF5"
  neutral-800: "#2B201A"
  neutral-600: "#6B594C"
  espresso-900: "#191210"
  matcha-600: "#2E7D46"
  taro-600: "#5B3E8A"
  # semantic light (see §2.2; .dark overrides in appendix)
  bg: "#FFFBF5"
  surface: "#FFFFFF"
  surface-muted: "#F5ECE0"
  fg: "#2B201A"
  fg-muted: "#6B594C"
  border: "#E4D3BF"
  primary: "#9A5418"
  on-primary: "#FFFFFF"
  accent: "#5B3E8A"
  success: "#2E6B3E"
  warning: "#8A5300"
  danger: "#A3281C"
typography:
  font-display: "Fredoka"
  font-body: "Nunito"
  font-mono: "JetBrains Mono"
  display-lg: { fontFamily: "Fredoka", fontSize: "40px", fontWeight: 600, lineHeight: "44px", letterSpacing: "-0.5px" }
  h1: { fontFamily: "Fredoka", fontSize: "28px", fontWeight: 600, lineHeight: "34px" }
  h2: { fontFamily: "Fredoka", fontSize: "22px", fontWeight: 600, lineHeight: "28px" }
  h3: { fontFamily: "Fredoka", fontSize: "18px", fontWeight: 600, lineHeight: "24px" }
  body: { fontFamily: "Nunito", fontSize: "16px", fontWeight: 400, lineHeight: "24px" }
  body-sm: { fontFamily: "Nunito", fontSize: "14px", fontWeight: 400, lineHeight: "20px" }
  caption: { fontFamily: "Nunito", fontSize: "13px", fontWeight: 600, lineHeight: "18px" }
  score: { fontFamily: "JetBrains Mono", fontSize: "28px", fontWeight: 600, lineHeight: "28px" }
rounded:
  xs: "6px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "20px"
  "2xl": "28px"
  pill: "999px"
spacing:
  "0": "0px"
  "1": "4px"
  "2": "8px"
  "3": "12px"
  "4": "16px"
  "5": "20px"
  "6": "24px"
  "8": "32px"
  "10": "40px"
  "12": "48px"
  "16": "64px"
  "20": "80px"
components:
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.on-primary}"
    rounded: "{rounded.md}"
    height: "40px"
    padding: "0 16px"
  card:
    backgroundColor: "{colors.surface}"
    rounded: "{rounded.lg}"
    padding: "{spacing.5}"
  input:
    backgroundColor: "{colors.surface}"
    borderColor: "{colors.border}"
    rounded: "{rounded.md}"
    height: "40px"
---

# Boba Job Find — Design System

> **Status:** final draft · **Owner:** ux · **Date:** 2026-10-04
> **Companions:** [`brand.md`](./brand.md) · [`voice-and-tone.md`](./voice-and-tone.md) · [`EXPERIENCE.md`](./EXPERIENCE.md) · [`preview.html`](./preview.html)
> **Verification:** every contrast pair below is checked by runnable `assets/palette_check.py` (`python3 assets/palette_check.py` → all pass, exits 1 on any failure).

This is the single source of truth for *how Boba Job Find looks*. Behavior lives in `EXPERIENCE.md`,
words in `voice-and-tone.md`. Tokens are **three-layer**: primitive → semantic → component. Components
must reference semantic or component tokens, **never raw hex** (one exception: the primitive ramp tables
themselves). If a value is missing here, add it to the system — do not hardcode it in a component.

Target implementation: **React + Tailwind / shadcn-style CSS custom properties** (final UI lib chosen by
architect — §10 ships the `:root` / `.dark` / `@theme` bridge, which works with shadcn, Tailwind v4, or
plain CSS). Icons: **Lucide** (ISC). Fonts: **Fredoka**, **Nunito**, **JetBrains Mono** — all
open-license (SIL OFL / Apache-2.0).

---

## 1. Brand & Style

**Direction:** *soft, rounded, warm, plain-spoken.* Derived from the boba metaphor (see `brand.md`):
a calm daily ritual rather than an urgent dashboard. Closest named style: **claymorphism, toned
down for desktop density** — soft double shadows, chunky radii, thick-ish borders, generous padding,
and *no* hard black lines. Playful in surface, rigorous in data.

**Design principles**

1. **Calm over urgent.** One obvious next step per screen. No streaks, no FOMO, no red-alert chrome.
2. **Warm honesty.** The surface is friendly; the scores never lie. A stretch is labeled a stretch.
3. **Data with a face.** Every number is explainable on demand (evidence, matched/missing, confidence).
4. **Atomic.** Sections are assembled only from the component inventory. New piece → added to the system.
5. **Yours.** Local-first language and UI: "on your machine", no accounts, nothing leaves the computer.
6. **Two scores, always visible.** The Fit Score and Quality Score are always shown alongside the
   Rank Score and its Verdict — the Rank never hides the two axes behind one opaque number.

**Tone/voice** (detail in `voice-and-tone.md`): *a warm, honest job-search coach who makes great boba.*

**Do not** import visual tropes of finance/gambling apps (glows, neon, confetti storms, countdowns).

---

## 2. Colors

### 2.1 Primitive tokens

Raw ramps. Components must not reference these directly — map through semantic tokens.

#### Caramel (brand / tapioca)
| Token | Hex | Token | Hex |
|---|---|---|---|
| `caramel-50` | `#FDF7F1` | `caramel-500` | `#C07430` |
| `caramel-100` | `#F7E7D3` | `caramel-600` | `#9A5418` |
| `caramel-200` | `#EFCFA8` | `caramel-700` | `#7E4312` |
| `caramel-300` | `#E3B27A` | `caramel-800` | `#5E3210` |
| `caramel-400` | `#D4904E` | `caramel-900` | `#3F220C` |

#### Warm neutrals (cream → espresso)
| Token | Hex | Token | Hex |
|---|---|---|---|
| `neutral-0` | `#FFFFFF` | `neutral-500` | `#8A7866` |
| `neutral-50` | `#FFFBF5` | `neutral-600` | `#6B594C` |
| `neutral-100` | `#F7F0E6` | `neutral-700` | `#4E4038` |
| `neutral-150` | `#F5ECE0` | `neutral-800` | `#2B201A` |
| `neutral-200` | `#EEE3D5` | `neutral-900` | `#191210` |
| `neutral-300` | `#DDCBB6` | `neutral-950` | `#14100E` |
| `neutral-400` | `#E4D3BF` | `border-strong-light` | `#C4AE94` |

#### Espresso (dark surfaces)
| Token | Hex | Token | Hex |
|---|---|---|---|
| `espresso-700` | `#3E2F27` | `espresso-900` | `#191210` |
| `espresso-800` | `#2E221C` | `espresso-950` | `#14100E` |
| `espresso-850` | `#241A16` | `border-strong-dark` | `#5A463A` |

#### Flavor ramps (semantic hues)
| Flavor | 100 (soft) | 300 (dark-mode text) | 600 (fill) | 700 (light-mode text) |
|---|---|---|---|---|
| **Matcha** — success, fit | `#E3F2E7` | `#8FD3A0` | `#2E7D46` | `#2E6B3E` |
| **Leaf** — good | `#EDF3D6` | `#B6D46A` | `#5A7A12` | `#4C660C` |
| **Mango** — warning, moderate | `#FCEFD2` | `#F0C070` | `#9E6203` | `#8A5300` |
| **Tangerine** — weak | `#FBE3D4` | `#F0A578` | `#B84E12` | `#9A3E10` |
| **Lychee** — danger, poor | `#FBE0DC` | `#F2A49B` | `#A82E24` | `#A3281C` |
| **Taro** — accent, info, quality | `#EFE6FA` | `#C4A6F0` | `#7A5AA6` | `#5B3E8A` |

### 2.2 Semantic tokens — **light**

| Token | Value | Use |
|---|---|---|
| `--bg` | `#FFFBF5` | app background |
| `--surface` | `#FFFFFF` | cards, panels, inputs |
| `--surface-muted` | `#F5ECE0` | zones, table stripes, hover fills |
| `--surface-sunken` | `#F0E4D5` | wells, skeletons, preview gutters |
| `--fg` | `#2B201A` | primary text |
| `--fg-muted` | `#6B594C` | secondary text |
| `--fg-subtle` | `#8A7866` | decorative / non-text only (see §12) |
| `--border` | `#E4D3BF` | default borders, dividers |
| `--border-strong` | `#C4AE94` | emphasized borders, input hover |
| `--primary` | `#9A5418` | brand, primary actions |
| `--on-primary` | `#FFFFFF` | text/icons on primary |
| `--primary-hover` | `#7E4312` | primary pressed/hover |
| `--primary-soft` | `#F7E7D3` | selected nav, brand tints |
| `--on-primary-soft` | `#7E4312` | text on primary-soft |
| `--accent` | `#5B3E8A` | links on accent surfaces, quality axis |
| `--on-accent` | `#FFFFFF` | text on accent |
| `--success` | `#2E6B3E` | success text/icon |
| `--warning` | `#8A5300` | warning text/icon |
| `--danger` | `#A3281C` | error/destructive text |
| `--info` | `#5B3E8A` | informational text |
| `--focus` | `#9A5418` | focus ring |

### 2.3 Semantic tokens — **dark**

| Token | Value | Notes |
|---|---|---|
| `--bg` | `#191210` | warm espresso, never pure black |
| `--surface` | `#241A16` | |
| `--surface-muted` | `#2E221C` | |
| `--surface-sunken` | `#14100E` | |
| `--fg` | `#F5EDE4` | |
| `--fg-muted` | `#BDAB9A` | |
| `--fg-subtle` | `#7E6B5C` | decorative only |
| `--border` | `#3E2F27` | |
| `--border-strong` | `#5A463A` | |
| `--primary` | `#E8A860` | lightened caramel for contrast on dark |
| `--on-primary` | `#241A16` | **dark text on light caramel** |
| `--primary-hover` | `#F0B878` | |
| `--primary-soft` | `#3A2A1E` | |
| `--on-primary-soft` | `#F0B878` | |
| `--accent` | `#C4A6F0` | |
| `--on-accent` | `#241A16` | |
| `--success` | `#8FD3A0` | |
| `--warning` | `#F0C070` | |
| `--danger` | `#F2A49B` | |
| `--info` | `#C4A6F0` | |
| `--focus` | `#E8A860` | |

### 2.4 WCAG AA verification

Every pair is asserted by `assets/palette_check.py`. Summary (ratios measured, threshold 4.5:1
normal text):

| Pair | Light | Dark |
|---|---:|----:|
| text on bg | 15.38 | 15.96 |
| text-muted on bg | 6.45 | 8.33 |
| text on surface | 15.86 | 14.68 |
| text-muted on surface | 6.65 | 7.66 |
| on-primary vs primary | 5.75 | 8.26 |
| link/accent on bg | 6.63 | 10.42 |
| danger text | 7.30 | 8.55 |
| success text | 6.39 | 9.72 |
| warning text | 6.33 | 10.11 |
| info text | 8.40 | 8.15 |

`--fg-subtle` (`#8A7866` light / `#7E6B5C` dark) does **not** meet 4.5:1 and is therefore
**non-text / decorative only** — never use it for body copy, labels, or icon-only controls that
carry meaning.

### 2.5 Theme switching

Themes follow the OS by default (`prefers-color-scheme`) with an explicit override in Settings:
`System → Light → Dark`. The override is persisted. All semantic tokens swap values; **no component
markup changes**. Ship tokens on `:root` (light) plus a `.dark` class on the root element (set by the
theme controller, which respects `prefers-color-scheme` when the mode is `System`) — see §14.

---

## 3. Score system (product-specific)

Vocabulary here is **fixed by the PRD glossary** (`docs/product/PRD.md` §3) — use these terms, no
synonyms. The ranking model (see `docs/research/offer-scoring.md`) emits two axis scores, a weighted
rank, a banded verdict, and hard gates. Color must never be the only carrier of meaning.

### 3.1 The scores

| Term | Range | Meaning |
|---|---|---|
| **Fit Score** | 0–100 | how well the Profile meets the Offer's requirements |
| **Quality Score** | 0–100 | how good the Offer is, regardless of whether the user can win it |
| **Rank Score** | 0–100 | weighted combination — default `0.65·Fit + 0.35·Quality` |
| **Verdict** | band | band of the **Rank Score**: Strong ≥75 · Good 60–74 · Moderate 45–59 · Weak 30–44 · Poor <30 |
| **Confidence** | High/Medium/Low | data coverage behind the scores |
| **Explanation** | — | per-factor breakdown + plain-language narration |

### 3.2 Axis identity (Fit / Quality)

| Axis | Hue | Icon (Lucide) |
|---|---|---|
| **Fit Score** | **matcha** | `crosshair` |
| **Quality Score** | **taro** | `sparkles` |

Axis hue tints the ring/border **only**. Magnitude is the ring fill fraction + the number, which is
always shown. Axis rings carry **no band word** — the band word belongs to the Verdict.

### 3.3 Verdict — the band (of Rank Score)

The Verdict is the only element that carries a band word. **Five redundant encodings** accompany it:
color + numeric Rank Score + band word + shape glyph + meter fill. Removing color must not remove any
information.

| Verdict | Range | Fill | On-color | Glyph | Text light | Text dark |
|---|---|---|---|---|---|---|
| **Strong** | 75–100 | `#2E7D46` | `#FFFFFF` | ● solid | `#2E6B3E` | `#8FD3A0` |
| **Good** | 60–74 | `#5A7A12` | `#FFFFFF` | ◗ three-quarters | `#4C660C` | `#B6D46A` |
| **Moderate** | 45–59 | `#9E6203` | `#FFFFFF` | ◐ half | `#8A5300` | `#F0C070` |
| **Weak** | 30–44 | `#B84E12` | `#FFFFFF` | ◔ one-quarter | `#9A3E10` | `#F0A578` |
| **Poor** | 0–29 | `#A82E24` | `#FFFFFF` | ○ outline + hatch | `#A3281C` | `#F2A49B` |

The ramp is **sequential** (green → olive → amber → orange → red). **Poor** additionally gets a subtle
diagonal hatch fill so it stays legible in grayscale. The **Verdict chip** reads `[● Strong · 78]`.

### 3.4 Meter primitives

- **Pips** (compact): 5 segments; count reflects the band + within-band position. Used for Verdict.
- **Ring** (detail): 44px compact / 64px detail; track `--surface-sunken`, fill = **axis hue**
  (matcha/taro), center = mono number. Fit ring and Quality ring sit side by side; the Rank Score is a
  separate Verdict chip, labeled *"Rank"*.
- **Bar** (Explanation rows): 4px track, fill tinted by the factor's own score band, 0–100, `pill`.

### 3.5 Confidence

Confidence is **not** a band. It is a separate `ConfidenceMeter` (dashed segments) with text labels
**High / Medium / Low**. When Confidence is **Low**, the Verdict is presented as a **"Lead"** (PRD
FR-57): a dashed outline, labeled **"Lead — worth checking"**, never shown as a hard band verdict.
Weights are user-visible and adjustable; the Rank chip tooltip states the current weights.

### 3.6 Gates

Hard gates run before scoring and return `PASS | FLAG | VETO` with **quoted evidence**:

| Verdict | Icon | Treatment |
|---|---|---|
| PASS | `check` | quiet, muted; rarely surfaced |
| FLAG | `alert-triangle` | warning tone, visible badge, Offer still ranked |
| VETO | `ban` | Offer excluded by default but **shown in the "Excluded" section with the quote and an override** (FR-54) |

A VETO is never a silent drop. Override is always one action away; the override is recorded.

### 3.7 Explanation

The per-factor breakdown (Fit F1–F6, Quality Q1–Q5) with `{score, weight, matched[], missing[],
evidence[], confidence}` plus a 2–4 sentence narration grounded only in those outputs (FR-59). The
narration is clearly labeled **narration**; the LLM may narrate but never sets or changes a number.

### 3.8 Accessibility of scores

Screen readers get a full sentence: *"Fit Score 82 out of 100. Quality Score 64 out of 100. Rank
Score 75 — Verdict Strong. Confidence medium."* The band word and numbers are in the accessibility
name; color is not announced as the value.

---

## 4. Typography

### 4.1 Families

| Role | Family | Weights | License | Fallback stack |
|---|---|---|---|---|
| Display / headings | **Fredoka** | 400/500/600/700 | SIL OFL | `Fredoka, "Baloo 2", ui-rounded, "Segoe UI", system-ui, sans-serif` |
| Body / UI | **Nunito** | 300/400/500/600/700 | SIL OFL | `Nunito, ui-rounded, "Segoe UI", Roboto, system-ui, sans-serif` |
| Numeric / data | **JetBrains Mono** | 400/500/600 | SIL OFL | `"JetBrains Mono", ui-monospace, "SF Mono", Menlo, monospace` |

**Self-hosted** via `@fontsource` (bundled `woff2`; subset `latin` + `latin-ext` for ES accents;
`font-display: swap`). **No Google Fonts CDN** — local-first and compatible with the app's
`Content-Security-Policy: default-src 'self'` (architect, 2026-10-04).

**Documents are not UI type.** Generated CVs / cover letters (Typst templates) render with **neutral,
ATS-safe fonts by default** — brand typefaces are never required in a Document, because ATS parsers
and other systems must read them. Fredoka/Nunito may be offered as an *optional* Template choice only
(FR-71), never the default.

### 4.2 Type scale

Fluid where the window resizes (`clamp`); fixed inside dense UI.

| Token | Size / line | Weight | Use |
|---|---|---|---|
| `--text-display-lg` | clamp(32px, 2.2vw + 18px, 40px) / 1.1 | Fredoka 600 | onboarding hero, empty-state headlines |
| `--text-display` | clamp(26px, 1.6vw + 14px, 32px) / 1.15 | Fredoka 600 | page titles |
| `--text-h1` | 28 / 34 | Fredoka 600 | section titles |
| `--text-h2` | 22 / 28 | Fredoka 600 | card group titles |
| `--text-h3` | 18 / 24 | Fredoka 600 | offer title in detail, modal titles |
| `--text-body-lg` | 16 / 24 | Nunito 400 | body default |
| `--text-body` | 15 / 22 | Nunito 400 | dense body |
| `--text-body-sm` | 14 / 20 | Nunito 400 | metadata, helper |
| `--text-caption` | 13 / 18 | Nunito 600 | labels, chips, captions |
| `--text-micro` | 12 / 16 | Nunito 700, tracking .02em | tags, overlines (UPPERCASE) |
| `--text-score-xl` | 40 / 40, tabular-nums | JetBrains Mono 600 | detail rings |
| `--text-score` | 28 / 28, tabular-nums | JetBrains Mono 600 | rings |
| `--text-score-sm` | 18 / 18, tabular-nums | JetBrains Mono 600 | compact rows |

**Rules:** body base is 16px; text never below 12px. Headings Fredoka, everything readable in Nunito.
All numerals in score/comp/meters use `font-variant-numeric: tabular-nums`. Max line length ~72ch for
prose; offer descriptions clamp to 3 lines with expand. Spanish text runs ~15–30% longer — all
truncation/pill layouts must survive +30% without clipping.

---

## 5. Layout & Spacing

### 5.1 Spacing scale (4px base)

`--space-0:0` · `1:4` · `2:8` · `3:12` · `4:16` · `5:20` · `6:24` · `8:32` · `10:40` · `12:48`
· `16:64` · `20:80` · `24:96` px.

Usage: 4 = icon↔label; 8 = chip gaps; 12 = dense rows; 16 = default control padding; 20–24 = card
padding / section rhythm; 32–48 = section separation; 64+ = onboarding hero breathing room.

### 5.2 Grid & containers

- Desktop content gutter: 24px (>=1280), 20px (1024–1279), 16px (<1024).
- Reading column max: **1200px**; offer list column max: **880px**; detail body max: **760px**.
- Offer grid: `repeat(auto-fill, minmax(320px, 1fr))` with 16px gap; single column <720px.
- Kanban: horizontal flex, column width clamp(280px, 24vw, 340px), 16px gap, sticky headers.
- App shell: see §9.

### 5.3 Motion & density

Two density modes: **Comfortable** (default, row 56px) and **Compact** (row 40px). Toggle in Settings
and via `Cmd/Ctrl+Shift+D`. Density changes spacing tokens, never type sizes below 12px.

---

## 6. Elevation & Depth

Soft, warm, layered — never hard black. Elevation is a semantic token (`--elev-*`) with light/dark
values.

| Token | Light | Dark | Use |
|---|---|---|---|
| `--elev-0` | none | none | flat surfaces, tables |
| `--elev-1` | `0 1px 2px rgba(63,34,12,.06), 0 1px 0 rgba(255,255,255,.6) inset` | `0 1px 2px rgba(0,0,0,.5)` | cards rest |
| `--elev-2` | `0 4px 12px rgba(63,34,12,.10)` | `0 6px 16px rgba(0,0,0,.55)` | card hover, dropdown |
| `--elev-3` | `0 12px 28px rgba(63,34,12,.14)` | `0 16px 36px rgba(0,0,0,.6)` | popover, tray panel |
| `--elev-4` | `0 24px 48px rgba(63,34,12,.20)` | `0 28px 56px rgba(0,0,0,.66)` | modal, command palette |

Depth is conveyed by **elevation + border + surface tone** together (dark mode relies more on border
than shadow). Never use elevation alone for focus/selection.

---

## 7. Shapes

Claymorphism-adjacent: chunky radii, thick-ish borders, no sharp corners.

| Token | Value | Use |
|---|---|---|
| `--radius-xs` | 6px | tags, tiny chips |
| `--radius-sm` | 8px | inputs inner, code |
| `--radius-md` | 12px | buttons, inputs, tiles |
| `--radius-lg` | 16px | cards, panels |
| `--radius-xl` | 20px | offer cards, modals |
| `--radius-2xl` | 28px | onboarding hero card, tray panel |
| `--radius-pill` | 999px | chips, meters, avatars |

Borders: default `1px solid var(--border)`; interactive hover `var(--border-strong)`; playful accent
elements (selected offer) may use a `2px` primary border. Never a pure-black border.

---

## 8. Motion

Motion *conveys meaning* (state change, spatial continuity), never decoration for its own sake.

| Token | Value | Use |
|---|---|---|
| `--dur-instant` | 80ms | hover tint, press |
| `--dur-fast` | 150ms | buttons, toggles, chips |
| `--dur-base` | 220ms | cards, expand/collapse, panel |
| `--dur-slow` | 320ms | drawer, modal, kanban move |
| `--dur-narrative` | 480ms | onboarding step, empty-state entrance |
| `--ease-standard` | `cubic-bezier(.2,0,0,1)` | most transitions |
| `--ease-emphasized` | `cubic-bezier(.05,.7,.1,1)` | enter, expand |
| `--ease-exit` | `cubic-bezier(.4,0,1,1)` | dismiss, collapse |
| `--ease-spring` | `cubic-bezier(.34,1.56,.64,1)` | pearl "pop" on new match — **sparingly** |

**Rules**
- **Exit faster than enter** (e.g. enter 220ms / exit 150ms).
- Animate `transform` + `opacity` (+ `background-color`), never `width`/`height`/`top`/`left`.
- Kanban drag uses transform + a raised `--elev-3`; drop settles with `--ease-emphasized` at 220ms.
- New-match pearl pop: scale 0.9→1 with `--ease-spring`, **once**, ≤480ms, only when the window is focused.

**Reduced motion** — under `prefers-reduced-motion: reduce` (and the in-app "Reduce motion" setting):
- All durations collapse to ≤120ms **crossfade**; transforms disabled.
- No spring, no confetti, no pearl pop, no parallax/scroll-scrub, no animated meters filling
  (values appear instantly in their final state).
- Loading uses a static/slow opacity pulse only.
- Focus/hover state changes remain (they are state, not decoration), just without movement.

---

## 9. Iconography

- **Set:** **Lucide** (ISC license, 1000+ icons, consistent 24px/2px stroke grid). Lucide is the
  default; missing glyphs are hand-drawn to the same grid.
- **Sizes:** `16` (dense inline), `20` (default UI), `24` (nav, prominent), `32` (empty states).
  Stroke `2` at 24px; `1.75` optical at 20px; never thinner.
- **Color:** inherits `currentColor`; semantic icons use semantic token.
- **Filled vs outline:** outline default; `filled` only for the **active** nav item and saved/starred.
- **No emoji as icons.** No icon-only control without an accessible name + tooltip.
- **Custom set (boba-specific):** score band glyphs (§3.2), axis icons (`crosshair`, `sparkles`),
  pearl mark, mascot Tapi. These are SVG assets in `assets/`.
- Key product icons: `home`, `search`, `inbox`, `kanban-square`, `sliders-horizontal`, `plug-2`
  (connections), `settings`, `send`, `file-text`, `download`, `refresh-cw`, `pause`, `play`,
  `bell`, `moon`, `sun`, `languages`, `shield-check`, `alert-triangle`, `ban`, `check`,
  `x`, `chevron-*`, `grip-vertical`, `external-link`, `sparkles`, `crosshair`, `pearl` (custom).

---

## 10. Component inventory

Bottom-up. Build order (atoms → molecules → organisms → sections) is also the handoff order to `dev`
(tracked in `.hermad/memory/ux-design.md`). Every component lists **states** and **responsive** rules.
Visual specs are here; behavioral specs (when/how) live in `EXPERIENCE.md` §Component Patterns.

### 10.1 Atoms

| Component | Key tokens | Variants | States |
|---|---|---|---|
| **Button** | `--primary`, `--radius-md`, h40 | primary, secondary, ghost, danger, link; sizes sm36/md40/lg44 | default, hover, active, focus, disabled, loading (spinner + label stays) |
| **IconButton** | 36×36 / 44×44 hit | ghost, soft, solid | + selected, tooltip required |
| **Input / Textarea / Select** | `--surface`, `--border`, `--radius-md` | text, search, select, number, date | default, hover, focus(ring), filled, disabled, error, success, read-only |
| **Checkbox / Radio / Switch** | `--primary` | — | default, hover, focus, checked, disabled, indeterminate (checkbox) |
| **Badge** | flavor 100/700 | neutral, success, warning, danger, info, accent | static |
| **Chip / Tag** | `--radius-pill` | filter (dismissible), meta, source | + selected, removable |
| **Avatar** | `--radius-pill` | image, initials, company | sizes 20/24/32/40 |
| **Meter** | band tokens | pips, bar, ring, dashed (confidence) | value 0–100, indeterminate |
| **Tooltip** | `--elev-3` | top/bottom/left/right | open/closed; 300ms delay; never the only label source |
| **Kbd** | `--surface-muted` | single, chord | static |
| **Skeleton** | `--surface-sunken` | line, block, circle, card | static / slow-pulse (respects reduced motion) |
| **Divider** | `--border` | horizontal, vertical, labeled | static |
| **Logo / Mascot** | assets | mark, mono mark, Tapi (8 poses) | static |

### 10.2 Molecules

| Component | Anatomy | Notes / states |
|---|---|---|
| **Field** | label + control + help + error | label always visible (never placeholder-only); error adjacent + `aria-describedby`; error, disabled, optional, required |
| **SegmentedControl** | 2–5 segments, sliding indicator | used for Fit/Quality toggle, density, theme; focus roving |
| **AxisScore** | ring + number + axis icon (Fit/Quality) | axis hue tints the ring; magnitude = fill + number; **no band word** on an axis; sizes 44/64; states: normal, lead(dashed), excluded |
| **VerdictChip** | band glyph + band word + Rank Score | the **only** element carrying a band word; color + number + word + glyph; states: normal, lead(dashed), excluded |
| **DualScore** | AxisScore(Fit) + AxisScore(Quality) + VerdictChip(Rank) | compact (pips+numbers) and detail (rings) forms |
| **ScoreBreakdownRow** | factor name · weight · bar · score · confidence dot | expand → matched[] / missing[] / evidence quote; states collapsed/expanded/low-confidence |
| **OfferMetaRow** | company · location chip · work-mode chip · comp · age | truncates; +30% ES safe |
| **ConnectionTile** | avatar + name + region + **Source Type** chip + **Risk Tier** badge + status pill + action | Risk Tiers: Safe · Opt-in · Prohibited-automation; see §10.3 |
| **WizardStep** | index/glyph + label | locked, current, done(check), error, optional/skipped |
| **ListRow** | leading + content + trailing | hover, selected, focus, dense |
| **FilterBar** | chips + query + sort + view toggle | sticky; shows active-count; clear-all |
| **InlineAlert / Banner** | icon + title + body + action | info/success/warning/danger; dismissible; `role=status/alert` |
| **Toast** | icon + message + action + close | auto-dismiss 6s (pause on hover/focus), queue, undo action |
| **EmptyState** | Tapi pose + headline + body + primary CTA (+ secondary) | one CTA max; used only for true empties |
| **CommandPaletteItem** | icon + label + hint + shortcut | highlight query match |
| **ProfileSectionEditor** | section card: fields + **evidence** chips (which role/project demonstrates a skill) | FR-11; edit/confirm; mirrors `ProfileSection` |
| **VariantToggle (EN/ES)** | segmented EN · ES + "draft the missing variant" affordance | FR-12; a Document in an unconfirmed variant warns before Send |
| **ImportReviewRow** | extracted field + value + **source excerpt** + Confirm/Edit/Delete | FR-5; nothing enters the Profile until confirmed |
| **VersionHistoryRow** | version no. + date + summary + Restore | FR-13; each Application Package records the version used |
| **StatusProposalCard** | email quote + proposed **Stage** change + confidence + **Approve** / Dismiss | FR-102; the quoted line is always shown |
| **OutboxRow** | Offer + Package summary + countdown + **Edit** / **Cancel** | FR-85; notified on entry |
| **RedFlagBadge / ScamBadge** | icon + label (Likely ghost · Recruitment-fraud) | FR-60; a fraud badge **blocks Auto** |
| **SourceHealthTile** | name + Source Type + **Risk Tier** + status + last run + Offers found + last error | FR-23/31; 3 fails → one notification |
| **AIUnavailableState** | inline disabled notice + "Connect an agent" CTA | FR-4 Limited mode; AI-only actions visibly disabled |
| **TaskUnsupportedNotice** | task name + chosen CLI + plain reason + "See supported CLIs" link | **P0-2 hard gate** — no "run anyway". A CLI without verified `tool_lockdown` is *unsupported* for the untrusted-content tasks (extract postings, classify email, tailor, review, narrate scores). Shown in Engine settings + inline where the task would run |
| **LockedBanner** | lock icon + "Boba is locked" + **Unlock** + why | **P1-5** — passphrase pending; DB unavailable. Topbar chip, tray line, and a skipped-run row. Never shows a run as successful while locked |

### 10.3 Organisms

#### OfferCard — the signature component

**Purpose:** present one opportunity with both axes legible at a glance and one obvious next step.

**Variants**
- **`row`** (Inbox, dense): 56/64px row — avatar, title, company, meta row, compact `DualScore`
  (two rings/pips + numbers + Verdict chip), flags, hover quick-actions (**Save**, **Dismiss**, **Tailor**).
- **`card`** (grid, Inbox card view): elevated card — title, company, meta, detail `DualScore` (rings),
  top "why" lines (best match + biggest gap), actions primary **Tailor** + secondary **Save**/**Dismiss**.
- **`mini`** (Tracker kanban card): title, company, Verdict chip, next-action label, drag handle.

**Anatomy (card):** `[avatar] [title + company + meta] [DualScore] [why lines] [flags] [actions]`

**States:** default, hover (elev-1→elev-2, 150ms), focus-visible, selected (2px primary border +
primary-soft tint), saved, dismissed (dimmed + undo; optional reason), applied (Applied badge),
excluded/veto (ban badge, muted, "Override" action), loading (skeleton card), low-Confidence
("Lead" dashed Verdict).

**Tokens:** `--card-bg`=surface, radius `--radius-xl`, pad `--space-5`, border `--border`.
Fit accent `matcha-600`, Quality accent `taro-600`.

**Responsive:** >=1280 shows rings inline right; 720–1279 stacks rings under meta; <720 rings move
below title, actions become icon-only with labels on long-press/hover.

#### ScoreBreakdown panel

Top: **Gates** strip (Pass/Flag/Veto badges + quoted evidence, expandable). Then two groups:
**Fit** (F1 Skills, F2 Experience, F3 Domain, F4 Location, F5 Education, F6 Language) and
**Quality** (Q1 Comp, Q2 Company, Q3 Terms, Q4 Legitimacy, Q5 Alignment). Each row: name, weight,
bar, score, confidence dot, chevron. Expanded row shows `matched[]` (green check) and `missing[]`
(gap marker) and `evidence_spans` as quotes with source. Footer: **confidence summary** and
"adjust weights" link. Never edits numbers — read-only truth; LLM narration is clearly labeled
"narration" and separate from the data.

**Responsive:** lives in the right inspector >=1280; becomes a full-height drawer / route <1280.

#### Kanban board (Tracker)

**Stages (default, PRD §3):** Saved · Preparing · Applied · Interviewing · Offer · Hired · Rejected ·
No Response · Withdrawn. The four terminal Stages (Hired, Rejected, No Response, Withdrawn) are
**collapsible** (FR-93); the user can reorder/hide columns, not rename the vocabulary.
**Column header:** Stage name, count, overflow menu. **Card:** `OfferCard mini` with a Verdict chip.
**Interaction:** drag (mouse) and keyboard move (`Cmd/Ctrl+←/→`). Moving to **Applied** requires a
submission evidence record (FR-88); if absent the card is marked **Needs check**. Every change is
logged (FR-93). **States:** default, drag-over, dragging, empty column (small inline hint, no Tapi),
full, needs-check. **Responsive:** horizontal scroll with sticky headers; below 900px a **List mode**
toggle groups by Stage — kanban is not forced on narrow windows.

#### Tailoring Studio (per Application)

Three zones: **left** = section outline + Template picker + Document language EN/ES + tone;
**center** = CV/cover-letter preview on a paper surface (A4) with **changed lines highlighted** and
**claim traceability** (FR-73 — any sentence without a linked Profile item is flagged);
**right** = reviewer findings (FR-72), keyword coverage & declared gaps (FR-75), ATS checks (FR-77),
no-fabrication status (FR-74).
Controls: Template picker, language (auto-matched to the Offer, overridable, FR-70), **diff toggle**
(previous draft ↔ current), regenerate, **Export PDF (ATS)** / **Export DOCX** (FR-78), **Send**.
**Pre-send checks** block *Ready/Send* on a failed validator or ATS check (in Auto mode; warns in
Review mode). **Document type is neutral/ATS-safe by default** (§4.1) — brand fonts are an optional
Template choice, never the default.
**States:** generating (skeleton paper + progress), ready, edits-pending, validator-blocked, ATS-warn,
An Application Package is immutable once sent (PRD §3).
**Responsive:** >=1440 three columns; 1024–1439 right panel collapses to a drawer; <1024 a
top tab switch (Preview / Findings / Outline).

#### Sources & Connections grid

Grid of **ConnectionTile**s grouped by **Source Type** (API · ATS Board · Feed · Manual · Custom ·
Session Board) and **Email** (Gmail **send-only (read via IMAP/BYO)**; Outlook/Microsoft Graph
**read+send**; IMAP/SMTP **read+send**). Each tile shows its
**Risk Tier** badge (Safe · Opt-in · Prohibited-automation), status pill, last run, enable/disable,
manage. Opt-in tiers require an explicit warning + confirmation; Session Board Auto needs a *second*
per-Source opt-in (FR-91) with the persistent **ToS risk** badge.
**Email capability copy/states:** the Gmail tile is **Send-only** by default via the bundled public
OAuth client (Google restricts Gmail read); **Read** is offered only through *Enable read (advanced)*
using the user's own OAuth client ID, or via the IMAP/.eml import path. Never present Gmail as
read+send. Tile capability labels: `Send-only · connected` · `Read + send · connected` ·
`Read off — connect your client ID` · `needs-reauth`. Outlook/Graph and IMAP/SMTP tiles are read+send.
**States:** not-connected, connecting, connected, running, error, degraded, needs-reauth, disabled,
opted-out. **Responsive:** 3 cols >=1280, 2 cols 768–1279, 1 col <768.

#### Onboarding wizard

Horizontal **WizardStep** stepper (Done / Current / Upcoming) + content panel + footer
(**Back** / **Next**, or **Finish**). Order follows FR-1…FR-10; resumable, progress saved each step:
1. **Language & theme** (FR-1) — EN/ES, theme follows OS.
2. **Agent CLI** (FR-2/3/4) — detect installed CLIs with version/path/health; pick one + model; test
   call. If none works, offer **Limited mode**.
3. **Profile import** (FR-5) — Master CV (PDF/DOCX/LinkedIn export); every extracted field reviewed
   with its source excerpt; nothing enters the Profile until confirmed.
4. **Profile gaps** (FR-6) — short questions for Gate/Tailoring fields (work eligibility, CEFR,
   notice period, optional writing sample); skippable (lowers Confidence).
5. **Preferences** (FR-7) — roles, seniority, **Salary Floor**, locations/timezones, work mode,
   languages, deal-breakers, excluded companies.
6. **Sources** (FR-8) — recommended Safe Sources pre-selected, Risk Tier shown; add custom.
7. **Email** (FR-9, optional) — Gmail (**send-only** default), Outlook/Graph and IMAP/SMTP
   (read+send); the step explains exactly what Boba reads vs sends, and that Gmail reply-reading
   needs the user's own client ID or IMAP/.eml.
8. **Apply Mode & schedule** (FR-10) — default **Review** (Auto cannot be enabled here), daily run
   time (default 08:00), launch-at-login default off.
9. **Recovery backup** (P0-1, **only on unstable-signing builds**) — mandatory, cannot be skipped:
   save the one-time **recovery key** + backup file so a future update can't lock the user out of
   encrypted data. Hidden on stably-signed builds. If the upgrade acceptance test later fails, this
   is also the **recovery screen** (paste key → restore).
10. **First run** — finishing starts a **Search Run** and opens the Inbox with live progress.
**States:** per-step valid/invalid (Next disabled + inline reason), optionally-skipped, error, resumed
("Pick up where you left off"), Limited mode, offline (local steps still work). Recovery-backup step:
unacknowledged (Next disabled), key-copied, backup-saved, hidden (stable signing). Target ≤ 15 min (SM-1).
**Responsive:** >=1024 horizontal stepper; <1024 compact "Step 3 of 9" + current label; single column.

#### Tray panel & notifications

- **OS notification (FR-45):** app icon, title ("3 Strong, 5 Good today"), body (top Offers), actions
  **Open** · **Run now**. Clicking opens the Inbox. No notification when nothing is new (unless the
  user enabled one); quiet hours respected.
- **Tray popover (FR-42):** run status (last run time / running / needs attention / Auto on-off),
  items that need the user, **Outbox** count, top 3 mini rows, footer **Open Boba** · **Run now** ·
  **Pause schedule** · **Kill Switch** (FR-86). Max height ~420px; scrolls.
**States:** idle, running (step label), done, 0-new ("Nothing new today — that's okay."), error
(plain message + retry), paused, Auto on/off, Kill Switch engaged. Respects OS "do not disturb" and
reduced motion.

#### Outbox & Kill Switch

- **Outbox** (FR-85): automatic sends wait a hold period (default 30 min, min 5; Session Board min 60).
  A row shows the Offer, the Package, the countdown, and actions **Review now** · **Cancel**. Notified
  when an item enters the Outbox.
- **Kill Switch** (FR-86): one control in tray, app header, and a global shortcut. Stops all automatic
  sending within 2 s, cancels in-flight ATS automation before final submit, moves Outbox items to the
  review queue, and stays engaged until turned off. **High-stakes → confirm-ladder on re-enable.**

#### Profile surface

Structured, editable Profile (FR-11…FR-13). Left = section outline (identity, experience, education,
skills + evidence, projects, certifications, languages w/ CEFR, work eligibility, writing style);
center = **ProfileSectionEditor** with evidence chips; header = **VariantToggle (EN/ES)** and a version
badge. **Master CV import** opens **ImportReviewRow**s with source excerpts — Confirm/Edit/Delete per
field; nothing saves until confirmed. **Version history** drawer lists **VersionHistoryRow**s with
Restore; every confirmed change creates a version and re-marks open Offer scores stale (FR-14).
**States:** empty (no CV imported yet), imported-unconfirmed, confirmed, variant-missing, re-scoring,
error (unreadable file). **Responsive:** outline collapses to a top tab < 1024.

#### Needs-you queue & Outbox (Today)

Two stacked queues on **Today**:
- **Needs you** — items requiring a decision: **StatusProposalCard**s from email (Approve / Dismiss,
  each with the quoted line, FR-102), Items bounced from Auto (CAPTCHA, missing-variant, failed check),
  and Applications in **Needs check** (no submission evidence, FR-88).
- **Outbox** — **OutboxRow**s for automatic sends inside the hold (default 30 min, min 5; Session
  Board min 60), with **Edit** / **Cancel** and a live countdown; a notification fires on entry.
**States:** empty (Outbox clear), counting-down, holding, cancelled, bounced, proposal-pending,
proposal-approved. Kill Switch empties the Outbox into the review queue and moves it here.

#### Auto Rules editor & dry run (Settings ▸ Apply)

**AutoRuleEditor** is scoped to **Source × Apply Channel** (FR-83): minimum Rank Score and Fit Score,
minimum Confidence, allow-FLAGs, required Verdict, daily cap (default 5, max 25), per-company cap,
active hours. Enabling Auto requires the confirmation dialog + checkbox; per Source the user opts in
to **Risk Tiers**; a **Prohibited-automation** Source (LinkedIn/Indeed-class) additionally requires
**ToSConfirmation** — a plain-language warning and typing the Source name (FR-91) — and then shows a
persistent **ToS risk** badge on the tile and on every **Auto Rule** that targets it.
**Dry run** (FR-84): the first N matches of a new rule render as **DryRunResultRow**s — "what we
*would* send" with reasons — before the rule activates.
**States:** no rules (Auto off), rule-disabled, active, dry-run-pending, dry-run-report, source-not-opted-in.
**Responsive:** editor is a right drawer ≥ 1280, a full route < 1280.

#### Small views & diagnostics

- **Run report** (FR-46): Sources queried, Offers fetched / new / duplicates / vetoed / ranked, agent
  tasks used, errors, duration. Linked from Today and the tray.
- **Audit Log** (FR-89): append-only table of every send, Auto decision (sent/skipped + rule + reasons),
  Kill Switch event, and Stage change, with timestamp and the exact Package; exportable (FR-110).
- **Network activity** (NFR-1): the destinations contacted recently (Sources, email provider, Agent CLI
  provider, FX/update if enabled). No telemetry.
- **Source health** (FR-31): **SourceHealthTile**s with **Risk Tier** badges, status (OK / Degraded /
  Failing), last run, Offers found, last error; a Source failing 3 runs → one notification.
All four are **read-only tables/lists**: sortable, keyboard-navigable, virtualized when long, with a
plain **empty state** and a **CSV export** where applicable. **States:** empty, loading, error,
filtered. **Responsive:** cards on narrow widths, table ≥ 1024.

#### Capability gating & locked state (P0-2 · P1-5)

**CLI task capability (P0-2).** Agent CLIs advertise capability tiers probed at runtime (AD-4/AD-6).
A CLI may run *any* task only if it is **supported for that task**; **untrusted-content tasks**
(extract postings, classify email, tailor, review, narrate scores) additionally require verified
**`tool_lockdown`** (native shell + web tools removed, MCP allowlist only). The UI never offers an
override.

- **Engine settings** renders a **task × CLI support matrix** (`Supported` · `Unsupported` ·
  `Untested → Test`); `Unsupported` cells use **TaskUnsupportedNotice** and are non-interactive.
- Where a task would render (Studio **Tailor**/**Review**, onboarding **First run**, email sync) that
  task shows an **AIUnavailableState**-style inline notice: "*Tailoring needs an agent with tool
  lockdown. {CLI} can't run this task — pick another CLI or use Limited mode.*" with **Change CLI**.
- Selecting an unsupported CLI for those tasks is impossible; choosing one shows the blocked notice,
  never a disabled-but-clickable control that fails later.
**States:** supported, unsupported, untested (probe available), probing, probe-failed. In **Limited
mode** every untrusted-content task is unsupported by definition.

**Locked store (P1-5).** When the key is passphrase-derived and the user hasn't unlocked this login
session, the store is locked. The shell shows a persistent **LockedBanner** (topbar chip + tray line)
and an **UnlockDialog** on any gated action; unlock is **once per login session**, cached in zeroized
memory with an idle timeout, then re-locks.
- A scheduled **Search Run** while locked is recorded **skipped** (never "success"), the tray/status
  shows "*Boba is locked — enter your passphrase to run*", and one OS notification fires (no repeat
  spam). Completing the unlock offers **Run now**.
- Tray menu, topbar, Settings ▸ Privacy, and the Run report all expose the locked/skipped state.
**States:** locked, unlocking, unlocked (session), idle-timeout re-lock, wrong-passphrase, unlocked-but-stale.

#### AppShell (section template)

Sidebar (left) + Topbar + content + optional Inspector (right). Composed only of the inventory above.

### 10.4 Desktop layout

**Window**
- Minimum: **960 × 640** (below this the app shows a "resize to continue" floor — no broken layouts).
- Default on first run: **1280 × 832**, centered; remembers last size/position per platform.
- Comfortable: **1440 × 900**. Ultrawide: content column caps at 1200 and centers; kanban grows.

**Shell grid**

```
┌──────────────┬───────────────────────────────────────────────┬───────────────┐
│ SIDEBAR      │ TOPBAR   (title · search · Run now · Kill Sw.) │ INSPECTOR     │
│ 240 / 64 rail├───────────────────────────────────────────────┤ (>=1280 only) │
│              │                                               │ Explanation / │
│ Today        │              MAIN CONTENT                     │ Studio /      │
│ Inbox        │              (max 1200, gutter 24)             │ application   │
│ Studio       │                                               │ 360px         │
│ Tracker      │                                               │               │
│ Profile      │                                               │               │
│ Connections  │                                               │               │
│ Settings     │                                               │               │
│ ── footer ── │                                               │               │
│ Tapi · run   │                                               │               │
└──────────────┴───────────────────────────────────────────────┴───────────────┘
```

**Breakpoints (window width)**

| Width | Shell behavior |
|---|---|
| < 960 | floor screen (ask to resize); if unavoidable, single column + bottom sheet nav |
| 960–1023 | sidebar → **64px icon rail**; inspector → overlay drawer; content gutter 16 |
| 1024–1279 | sidebar rail (hover-expand); kanban horizontal scroll; inspector overlay |
| 1280–1439 | sidebar expanded 240; inspector inline 360 |
| 1440–1599 | + content max 1200; offer grid 3 col |
| >= 1600 | content centered, kanban columns grow; no new nav |
| ultrawide (>= 1920) | optional 3-pane (list · detail · breakdown) — content capped, whitespace intentional |

**Sidebar nav items:** Today (`home`), Inbox (`inbox`), Studio (`file-text`), Tracker
(`kanban-square`), Profile (`user-round`), Connections (`plug-2`), Settings (`sliders-horizontal`).
Footer: Search Run status chip, theme toggle, language toggle, Tapi idle. Active item = primary-soft
bg + filled icon + on-primary-soft text.

**Topbar:** current page title, global search (opens palette), **Run now** button (disabled +
tooltip while running), **Kill Switch** (only when Auto is on), run status ("Last run 06:12 · 12 new").
When the store is locked, a **LockedBanner** chip ("Locked — Unlock") replaces the run status until
unlocked (P1-5).

**Keyboard shortcuts** (all customizable; `Cmd` = `Cmd`/`Ctrl` per OS):

| Shortcut | Action |
|---|---|
| `Cmd/Ctrl+K` | Command palette / global search |
| `Cmd/Ctrl+1…7` | Jump to Today / Inbox / Studio / Tracker / Profile / Connections / Settings |
| `G` then `T/I/S/K/P/C/,` | Go to (mouseless nav) |
| `/` | Focus search |
| `J` / `K` | Next / previous Offer |
| `Enter` | Open focused Offer |
| `C` | Tailor in Studio for focused Offer |
| `A` | Apply (opens Review unless an Auto Rule matches) |
| `S` | Save (creates an Application in Saved) |
| `X` | Dismiss (optional reason) |
| `Cmd/Ctrl+Enter` | Confirm **Send** in Review |
| `Cmd/Ctrl+Shift+K` | **Kill Switch** (stop all automatic sending) |
| `Cmd/Ctrl+←/→` | Move focused Tracker card between Stages |
| `Esc` | Close palette/modal/drawer |
| `Cmd/Ctrl+,` | Settings |
| `Cmd/Ctrl+Shift+L` | Toggle light/dark (overrides system) |
| `Cmd/Ctrl+Shift+D` | Toggle density |
| `?` | Shortcut cheat-sheet |
| `Space` | Toggle multi-select in lists |

---

## 11. i18n & platform notes

- **Languages:** EN + ES from day 1. All strings in message catalogs (no hardcoded copy). Spanish
  uses **tú** (warm, neutral LATAM/ES). Numbers/dates/currency via `Intl`, locale-aware.
- **Text expansion:** design every pill/button/row for **+30%** string length without clipping
  (ES runs longer). Truncate only where a tooltip/title reveals the full value.
- **Platform:** macOS (traffic-light inset top-left → 28px top padding on the window header),
  Windows/Linux (custom window controls right). Tray icon uses the mono mark; macOS template image.
  Native notifications via Tauri notification API. Respect OS accent? **No** — brand primary is fixed.
- **Fonts bundled**, no network at runtime.

---

## 12. Accessibility floor

Non-negotiable. Any component failing these is not shippable.

1. **Contrast:** normal text ≥4.5:1, large text/UI ≥3:1 — verified in §2.4. `--fg-subtle` is
   decorative only.
2. **Never color alone:** every score carries number + word + glyph; gates carry icon + label;
   statuses carry icon + text; kanban stages show labels.
3. **Focus:** visible ring `2px solid var(--focus)` + `2px` offset on **all** interactive elements;
   never `outline: none` without a replacement. Focus order follows reading order.
4. **Targets:** primary controls ≥44×44; dense secondary ≥24×24 with ≥8px spacing (WCAG 2.2 min).
5. **Keyboard:** every flow completable without a mouse; kanban movable by keyboard; palette and
   modals trap focus and restore it on close; `Esc` always closes.
6. **Screen readers:** live regions for run status and toasts (`role=status`, `aria-live=polite`;
   errors `role=alert`); scores announced as full sentences (§3.8); icon-only buttons named.
7. **Motion:** `prefers-reduced-motion` + in-app setting honored (§8).
8. **Zoom:** usable at 200% (text survives, no loss; no horizontal scroll on content columns).
9. **Forms:** visible labels, errors adjacent with `aria-describedby`, inline validation on blur,
   no placeholder-only labels.
10. **Language:** `lang` set correctly per locale; language switch announced.

---

## 13. Do's and Don'ts

**Do**
- Reference semantic/component tokens; add a token rather than hardcode.
- Show **both** scores and let the user open the *why*.
- Use one primary action per view; make the next step obvious.
- Use Tapi at emotional beats; keep data screens clean.
- Design light and dark **together**; check both in every PR.
- Keep warmth in the surface and rigor in the numbers.

**Don't**
- Don't show the Rank Score/Verdict without the Fit Score and Quality Score beside it.
- Don't rely on color alone for any state (scores, gates, statuses).
- Don't use pure black/white surfaces; keep the warm cast (espresso, cream).
- Don't animate layout properties; don't use spring/confetti unless a new match lands.
- Don't put `--fg-subtle` on text; don't use emoji as icons.
- Don't hide VETOs; show the evidence and the override.
- Don't fork a near-duplicate component — extend the system.

---

## 14. Appendix — implementation bridge

Target path in the app: **`src/styles/tokens.css`** (Tailwind v4 + shadcn/Radix, per architect).
Shape (CSS custom properties + Tailwind v4 `@theme` bridge):

```css
:root {
  /* primitives */
  --caramel-600: #9A5418; --caramel-700: #7E4312;
  --cream-50: #FFFBF5; --neutral-150: #F5ECE0; --neutral-800: #2B201A;
  --neutral-600: #6B594C; --border-light: #E4D3BF; --border-strong-light: #C4AE94;

  /* semantic — light */
  --bg: var(--cream-50);      --surface: #FFFFFF;      --surface-muted: var(--neutral-150);
  --surface-sunken: #F0E4D5;  --fg: var(--neutral-800);--fg-muted: var(--neutral-600);
  --fg-subtle: #8A7866;       --border: var(--border-light); --border-strong: var(--border-strong-light);
  --primary: var(--caramel-600); --on-primary: #FFFFFF; --primary-hover: var(--caramel-700);
  --primary-soft: #F7E7D3;    --on-primary-soft: var(--caramel-700);
  --accent: #5B3E8A;          --on-accent: #FFFFFF;
  --success: #2E6B3E;         --warning: #8A5300;       --danger: #A3281C; --info: #5B3E8A;
  --focus: var(--caramel-600);

  /* bands */
  --band-strong: #2E7D46; --band-good: #5A7A12; --band-moderate: #9E6203;
  --band-weak: #B84E12;   --band-poor: #A82E24;
  --axis-fit: #2E7D46;    --axis-quality: #5B3E8A;

  /* radius / space / motion / elev */
  --radius-md: 12px; --radius-lg: 16px; --radius-xl: 20px; --radius-pill: 999px;
  --space-2: 8px; --space-4: 16px; --space-5: 20px; --space-6: 24px;
  --dur-fast: 150ms; --dur-base: 220ms; --ease-standard: cubic-bezier(.2,0,0,1);
  --elev-1: 0 1px 2px rgba(63,34,12,.06), 0 1px 0 rgba(255,255,255,.6) inset;
  --elev-2: 0 4px 12px rgba(63,34,12,.10);
}

.dark {
  --bg: #191210; --surface: #241A16; --surface-muted: #2E221C; --surface-sunken: #14100E;
  --fg: #F5EDE4; --fg-muted: #BDAB9A; --fg-subtle: #7E6B5C;
  --border: #3E2F27; --border-strong: #5A463A;
  --primary: #E8A860; --on-primary: #241A16; --primary-hover: #F0B878;
  --primary-soft: #3A2A1E; --on-primary-soft: #F0B878;
  --accent: #C4A6F0; --on-accent: #241A16;
  --success: #8FD3A0; --warning: #F0C070; --danger: #F2A49B; --info: #C4A6F0;
  --focus: #E8A860;
  --elev-1: 0 1px 2px rgba(0,0,0,.5);
  --elev-2: 0 6px 16px rgba(0,0,0,.55);
}

@theme inline {           /* Tailwind v4 bridge */
  --color-bg: var(--bg);
  --color-surface: var(--surface);
  --color-fg: var(--fg);
  --color-muted: var(--surface-muted);
  --color-primary: var(--primary);
  --color-accent: var(--accent);
  --color-success: var(--success);
  --color-warning: var(--warning);
  --color-danger: var(--danger);
  --radius-card: var(--radius-xl);
  --font-display: "Fredoka", ui-rounded, system-ui, sans-serif;
  --font-sans: "Nunito", ui-rounded, system-ui, sans-serif;
  --font-mono: "JetBrains Mono", ui-monospace, monospace;
}
```

Tailwind class usage: `bg-surface text-fg border-border rounded-card font-display`. Never
`bg-[#9A5418]` or `text-[#2B201A]`.

**Naming convention:** primitives `<hue>-<step>`; semantic `<purpose>`; component
`--<component>-<property>`. Theme-swap only ever happens at the semantic layer.
