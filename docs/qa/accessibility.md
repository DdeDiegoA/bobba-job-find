# Accessibility QA — keyboard, screen reader, zoom and reduced motion (S10.6)

Covers **NFR-15** (WCAG 2.2 AA) and the accessibility floor in **EXPERIENCE §7** for
the seven AC1 surfaces: the **wizard**, **Today**, **Offer detail**, **Studio**,
**Tracker**, **Applying** (Settings → Applying) and the **Kill Switch**.

The automated half lives in `e2e/a11y/` and runs with `pnpm test`. The half a test
cannot decide — what a real screen reader says, what 200% zoom looks like, what
motion feels like — is a scripted manual pass, in [Manual scripts](#manual-scripts).

## How to re-run

```sh
pnpm install                 # a fresh worktree needs it before any script
pnpm test -- e2e/a11y        # the six automated files (~5s)
```

| File | What it proves |
|---|---|
| `axe.ts` | axe-core (4.10.2, MPL-2.0, devDependency) over the same rendered DOM: WCAG 2.2 A/AA rules for ARIA validity, roles, list/table structure and duplicates. `color-contrast` is disabled here — jsdom has no layout — and covered by `contrast.test.ts` instead. |
| `screens.test.tsx` | Each AC1 screen, rendered with real components over a faked IPC seam, has an accessible name on every control, labelled fields, ordered headings, unique ids, resolvable `aria-labelledby`, no positive tabindex, no focusable element under `aria-hidden`, and no nested interactive controls. Also: the Auto confirmation opens with a labelled checkbox, Verdicts and Stages render as words, the Kill Switch carries `aria-keyshortcuts`. |
| `keyboard.test.tsx` | The skip link targets `<main tabindex="-1">`; navigation links are native links; the Kill Switch is a native focusable button that activates; a Tracker card moves with Cmd/Ctrl + ←/→; the wizard takes focus into its dialog on open. |
| `contrast.test.ts` | Every normal-text token pair the app uses is ≥ 4.5:1 in both themes, re-derived from `src/styles/tokens.css` (not from the design mock). |
| `reduced-motion.test.ts` | The global `prefers-reduced-motion: reduce` rule exists, covers `*`/`::before`/`::after`, cuts durations to a frame, disables smooth scroll, and is not overridden with `!important`. |
| `zoom.test.ts` | Pins the shell's 960 px floor against the window's 940 px minimum and the 200% zoom arithmetic (finding A11Y-6). |

The hand-written rules in `audit.ts` stay alongside axe: they state the floor in
terms this repo controls (and say *why* each one exists), while axe widens the net.
**axe-core is a devDependency only — it never ships in the app — and is licensed
MPL-2.0** (file-level copyleft; it is not linked into the product).

Findings that belong to another slice are pinned, never fixed here (S10.6 AC3):
`KNOWN` in `screens.test.tsx` suppresses exactly one violation per follow-up, with
the owner's ID in the comment, so it cannot grow silently and disappears the day
the owner ships the fix.

## What was executed, and what was not

Honest status for this pass (2026-10-17):

| Method | Status | Where |
|---|---|---|
| Automated DOM audit (7 screens) | **Executed** — 0 unexpected violations | `screens.test.tsx` |
| Keyboard semantics (focusability, handlers, skip link) | **Executed** as far as jsdom can decide | `keyboard.test.tsx` |
| Contrast (shipped tokens, both themes) | **Executed** | `contrast.test.ts` |
| Reduced motion (CSS contract) | **Executed** | `reduced-motion.test.ts` |
| 200% zoom (static arithmetic) | **Executed**; hands-on visual pass **not run** | `zoom.test.ts` + script D |
| Keyboard-only hands-on pass | **Not run** — no interactive GUI session in this QA environment | Script A |
| VoiceOver (macOS) | **Not run** — same | Script B |
| NVDA (Windows) | **Not run** — no Windows host here | Script C |

jsdom cannot paint focus rings, run the browser's implicit Enter/Space activation,
move focus on Tab, or expose an accessibility tree to a screen reader; nothing in
the automated suite claims any of those. The release checklist must carry scripts
A–D until a host with a screen reader runs them once per release.

## Results by surface

| Surface | Automated audit | Verdicts / Stages as text | Keyboard | Screen reader | 200% zoom |
|---|---|---|---|---|---|
| Wizard | clean | step counter is text (`Step N of M`), `aria-current="step"` | focus starts in the dialog, Tab is trapped (S3.1 tests) | script B/C | script D |
| Today | clean except A11Y-3 | band counts + Verdict words, but read as "Strong78" (A11Y-5) | links native | script B/C | **A11Y-6** |
| Offer detail | A11Y-1 | Verdict + Confidence words | Fit/Quality are buttons with hints | script B/C | **A11Y-6** |
| Studio | clean | tab text; checks are words | native controls | script B/C | **A11Y-6** |
| Tracker | A11Y-2 | Stage column headings are text | Cmd/Ctrl + ←/→ moves a card | script B/C | **A11Y-6** |
| Applying | clean | Auto/Review state is a word | confirmation is a labelled checkbox | script B/C | **A11Y-6** |
| Kill Switch | clean | engaged state is `role="status"` text | native button + `aria-keyshortcuts` | script B/C | **A11Y-6** |

Reduced motion: the global rule covers every animated element (skeletons,
spinners, dnd-kit is JS transform only). Content appears in its final state.
Hands-on confirmation is script E.

## Findings

Filed against the slice that owns the screen, for the orquestador to schedule as
follow-up slices. This pass changed nothing under `src/`.

| ID | Severity | Surface | Owner | Summary |
|---|---|---|---|---|
| **A11Y-1** | High | Offer detail | S6.3 | `aria-labelledby="offer-title"` points at an id that does not exist — the page has no name |
| **A11Y-2** | High | Tracker | S9.1 | Each card is a `role="button"` containing another `<button>` |
| **A11Y-3** | Medium | Today | S6.4 | "Open Inbox" is a `<Button>` inside a `<Link>` (a button inside a link) |
| **A11Y-4** | High (systemic) | 32 files | UX + per-slice | Text painted in `--fg-subtle`, which DESIGN.md reserves for decoration; it measures 4.11:1 (light) / 3.65:1 (dark) |
| **A11Y-5** | Medium | Today | S6.4 | Verdict and Rank run together in the accessibility tree ("Strong78") |
| **A11Y-6** | High | Shell, all screens | S1.5 + DESIGN §10.4 | Below 960 CSS px the shell is replaced by a resize notice; the window itself opens at 940 px, and 200% zoom on a 1440-px display lands at 720 px |
| **A11Y-7** | Medium | Applying | S9.6 / S9.7 | The Auto confirmation declares `aria-modal="true"` but neither moves focus nor traps it |
| **A11Y-8** | Low | Tracker, Studio | S9.1 / S7.7 | `role="tab"` buttons with no `aria-controls`, no tabpanel and no arrow-key handling |

### A11Y-1 — Offer detail has a dangling label reference

`src/features/offer/index.tsx:65` names the page section with
`aria-labelledby="offer-title"`, but `Header` renders the `<h1>` without an id
(`src/features/offer/sections.tsx:316`). Screen readers announce an unnamed region.

**Fix:** `id="offer-title"` on the `<h1>` (one attribute).
**Evidence:** `e2e/a11y/screens.test.tsx` — `KNOWN` entry A11Y-1.

### A11Y-2 — Tracker cards nest a button inside a button

`src/features/tracker/board/card.tsx` spreads dnd-kit's `attributes` onto the
`<li>`, which gives it `role="button"` and `tabindex="0"`; the card's own open
control is a real `<button>` inside it. Two activation targets share one name
family, and NVDA/VoiceOver report a button inside a button (WCAG 4.1.2).

**Fix:** take the drag handle off the list item's role — e.g. keep the `<li>`
presentational and give the drag handle (or the open button) the keyboard
affordances, or move the open action onto a single control.
**Evidence:** `KNOWN` entry A11Y-2; the duplicate query in `keyboard.test.tsx`
(`findAllByRole` matching two nodes) is the same defect.

### A11Y-3 — Today nests a button inside a link

`src/features/today/matches/index.tsx` renders
`<Link to="/inbox"><Button>Open Inbox</Button></Link>`. Invalid containment, and
several screen readers announce only one of the two controls.

**Fix:** style the link like a button (`Button asChild` + `Link`) instead of
wrapping.

### A11Y-4 — `--fg-subtle` is painted on text

DESIGN.md §2.4 (line 239) is explicit: `--fg-subtle` is 4.11:1 light / 3.65:1 dark
and is **decorative only — never body copy, labels or meaningful icons**. The app
still uses `text-fg-subtle` for visible text in **32 files / 78 occurrences**
(inbox cards, tracker detail, offer sections, the palette, the wizard steps,
`empty-state`, the topbar, …). Normal-size text at those ratios fails WCAG AA.

**Fix (per owner):** swap every text use to `text-fg-muted` (6.45:1 light /
8.33:1 dark) and keep `--fg-subtle` for dividers and glyphs, or darken the token
(which would make it the same as `--fg-muted`). The mechanical part is one chore;
the design call belongs to UX.
**Evidence:** `it.todo` pairs in `contrast.test.ts` (A11Y-4) hold the two failing
pairs until the owner's fix moves them into `PAIRS`.

### A11Y-5 — "Strong78" is read as one word

`src/features/today/matches/index.tsx` puts the Verdict word and the Rank number
in one span separated only by layout (`gap-1`). Text content is `Strong78`; a
screen reader says it as one token.

**Fix:** a space or a visually-hidden separator (`{" "}`, `<span className="sr-only">·</span>`).
**Evidence:** `it.fails` in `screens.test.tsx` (A11Y-5) — delete the pin when the
space lands.

### A11Y-6 — 200% zoom replaces the app with a resize notice

`src/styles/index.css` hides `.shell` at `max-width: 959.98px` and shows
`.resize-floor` instead. `src-tauri/tauri.conf.json` allows the window down to
**940 px**, already inside that range. At 200% zoom a 1440-px window becomes
720 CSS px: content and function are lost, which EXPERIENCE §7 and NFR-15 forbid.

**Fix (owner S1.5 with DESIGN §10.4):** either lower the floor to a width that
reflows (the board already scrolls horizontally), or keep the floor and raise
`minWidth` above it *and* exclude webview zoom — the current pair is wrong in
both directions.
**Evidence:** `zoom.test.ts` (3 passing assertions + 1 `it.fails` pin).

### A11Y-7 — The Auto confirmation claims to be a modal but is not one

`src/features/settings/applying/rules/index.tsx` (around line 153) renders the
"Turn on auto-apply?" panel with `role="dialog" aria-modal="true"` inline in the
page: focus stays on the trigger, Tab walks the whole screen, and the content
behind is not inert. `aria-modal="true"` promises the opposite to assistive tech.

**Fix:** either make it a real modal (move focus in, trap Tab, restore focus —
the wizard and palette already implement this) or drop `aria-modal` and keep it
as a labelled region. Given the confirmation's weight (Auto starts sending), the
modal is the right shape.

### A11Y-8 — Tab lists are tabs in name only

Tracker (`src/features/tracker/index.tsx`) and Studio (`src/features/studio/index.tsx`)
render `role="tablist"` with `role="tab"` buttons but no `aria-controls`, no
`role="tabpanel"`, no roving tabindex and no ←/→ handling. They remain operable
(every tab is in the Tab order), so this is a pattern-compliance finding, not a
blocker.

## Manual scripts

Run each script once per release candidate. Record pass/fail per surface in the
release notes. "Expected" describes the floor, not the polish.

### A. Keyboard only (any OS, ~30 min)

Unplug the mouse. For each surface: reach it from the sidebar/tray, complete its
primary action, and leave it — using only Tab, Shift+Tab, arrows, Enter/Space,
Escape and the documented shortcuts.

- **Wizard:** Tab reaches every control in the step; focus is trapped in the
  dialog; Next/Back work; the step counter is announced when it changes.
- **Today:** Tab reaches the Inbox link and each top match; Enter opens the Offer.
- **Offer detail:** Fit and Quality are reachable and scroll to their factor group.
- **Studio:** CV/cover tabs switch with Enter; the editor is reachable; Export is
  reachable; the apply bar's controls are all reachable.
- **Tracker:** Tab reaches a card; Cmd/Ctrl + ←/→ moves it; the confirmation for
  Applied is keyboard-operable; the stage collapse buttons are `aria-expanded`.
- **Applying:** Enable Auto opens the confirmation, the checkbox toggles with
  Space, the confirm button is disabled until checked.
- **Kill Switch:** reachable in the header and via Cmd/Ctrl+Shift+K; Enter engages
  it; the engaged chip is announced (`role="status"`).

Expected: no focus trap (except the wizard), no lost focus after an action, a
visible focus ring on every stop.

### B. VoiceOver (macOS, ~30 min)

1. System Settings → Accessibility → VoiceOver → on (or Cmd+F5).
2. `pnpm tauri dev`, complete the wizard with `Ctrl+Option+←/→` and `Ctrl+Option+Space`.
3. On each surface, use `Ctrl+Option+U` (rotor) → Headings: the page has one h1,
   and Today/Tracker announce their panels/columns as headings.
4. Confirm the Verdict is spoken as a word ("Strong"), the Rank as a number, and
   the Stage of a Tracker column as its heading.
5. Trigger a run and confirm the progress text (`role="status"`) is announced.

### C. NVDA (Windows, ~30 min)

Same script as B on a Windows host, with NVDA + Firefox? No — with the installed
app (WebView2). Use `NVDA+↓` to read by line, `H` for headings, `B` for buttons.

### D. 200% zoom (any OS, ~15 min)

1. Open the app at 1440 px logical width.
2. Zoom the webview (if enabled) or set the display scale to 200%.
3. For each surface: content is visible, no content column scrolls horizontally,
   every action remains reachable.

Expected today: **A11Y-6** — the resize notice replaces the app. This script is
the acceptance test for that finding's fix.

### E. Reduced motion (~10 min)

1. macOS: System Settings → Accessibility → Display → Reduce motion; Windows:
   Settings → Accessibility → Visual effects → Animation effects off; GNOME:
   Settings → Accessibility → Seeing → Reduce animation.
2. Open Today, start a run, open the Studio and drag a Tracker card.
3. Expected: no spinner rotation, no skeleton pulse, no smooth scroll — content
   appears in its final state. (dnd-kit's drag transform follows the pointer; it
   is direct manipulation, not animation.)
