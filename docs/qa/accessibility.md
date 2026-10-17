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
| `screens.test.tsx` | Each AC1 screen, rendered with real components over a faked IPC seam, has an accessible name on every control, labelled fields, ordered headings, unique ids, resolvable `aria-labelledby`, no positive tabindex, no focusable element under `aria-hidden`, and no nested interactive controls. Also: the Auto confirmation opens with a labelled checkbox and traps focus, Verdicts and Stages render as words, tabs expose their panels, the Kill Switch carries `aria-keyshortcuts`. |
| `keyboard.test.tsx` | The skip link targets `<main tabindex="-1">`; navigation links are native links; the Kill Switch is a native focusable button that activates; a Tracker card moves with Cmd/Ctrl + ←/→; the wizard takes focus into its dialog on open. |
| `contrast.test.ts` | Every normal-text token pair the app uses is ≥ 4.5:1 in both themes, re-derived from `src/styles/tokens.css` (not from the design mock) — including `--fg-subtle` on all five backgrounds it lands on. |
| `reduced-motion.test.ts` | The global `prefers-reduced-motion: reduce` rule exists, covers `*`/`::before`/`::after`, cuts durations to a frame, disables smooth scroll, and is not overridden with `!important`. |
| `zoom.test.ts` | The shell never hides; it reflows into a compact layout below 960 px, which is where the 940-px window minimum and 200% zoom (720 CSS px on a 1440-px window) both land (A11Y-6). |

The hand-written rules in `audit.ts` stay alongside axe: they state the floor in
terms this repo controls (and say *why* each one exists), while axe widens the net.
**axe-core is a devDependency only — it never ships in the app — and is licensed
MPL-2.0** (file-level copyleft; it is not linked into the product).

Findings that belong to another slice are pinned, never fixed here (S10.6 AC3):
`KNOWN` in `screens.test.tsx` suppresses exactly one violation per follow-up, with
the owner's ID in the comment, so it cannot grow silently and disappears the day
the owner ships the fix. CH14 fixed A11Y-1..8, so `KNOWN` is now empty; the
allowance and its matching assertions stay in place for the next pass.

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
| Wizard | clean | step counter is text (`Step N of M`), `aria-current="step"` | focus starts in the dialog, Tab is trapped (S3.1 tests) | script B/C | compact shell |
| Today | clean (A11Y-3, A11Y-5 fixed) | band counts + Verdict words, read as "Strong 78" | links native | script B/C | compact shell |
| Offer detail | clean (A11Y-1 fixed) | Verdict + Confidence words | Fit/Quality are buttons with hints | script B/C | compact shell |
| Studio | clean (A11Y-8 fixed) | tab text; checks are words; tabs expose their panel | native controls + arrow-key tabs | script B/C | compact shell, panels stack |
| Tracker | clean (A11Y-2, A11Y-8 fixed) | Stage column headings are text | grip button drags; Cmd/Ctrl + ←/→ moves a card; arrow-key tabs | script B/C | compact shell |
| Applying | clean (A11Y-7 fixed) | Auto/Review state is a word | confirmation moves focus in, traps Tab, Esc closes, focus returns | script B/C | compact shell |
| Kill Switch | clean | engaged state is `role="status"` text | native button + `aria-keyshortcuts` | script B/C | compact shell |

Reduced motion: the global rule covers every animated element (skeletons,
spinners, dnd-kit is JS transform only). Content appears in its final state.
Hands-on confirmation is script E.

## Findings — all fixed in CH14 (2026-10-17)

Filed by S10.6 against the slice that owned each screen. CH14 fixed all eight
and turned each one into a passing assertion; nothing is pinned as a known
failure any more (`KNOWN` in `screens.test.tsx` is empty).

| ID | Severity | Surface | Owner | Summary | Fix shipped |
|---|---|---|---|---|---|
| **A11Y-1** | High | Offer detail | S6.3 | `aria-labelledby="offer-title"` points at an id that does not exist — the page has no name | `id="offer-title"` on the `<h1>` |
| **A11Y-2** | High | Tracker | S9.1 | Each card is a `role="button"` containing another `<button>` | dnd-kit handles moved to a grip button; `<li>` is a plain list item |
| **A11Y-3** | Medium | Today | S6.4 | "Open Inbox" is a `<Button>` inside a `<Link>` (a button inside a link) | one `<Link>` styled with `buttonVariants` |
| **A11Y-4** | High (systemic) | 32 files | UX + per-slice | Text painted in `--fg-subtle` measured 4.11:1 (light) / 3.65:1 (dark) | token re-tuned to `#756249` / `#A6907F`; all text pairs ≥ 4.5:1 |
| **A11Y-5** | Medium | Today | S6.4 | Verdict and Rank run together in the accessibility tree ("Strong78") | a real space between them ("Strong 78") |
| **A11Y-6** | High | Shell, all screens | S1.5 + DESIGN §10.4 | Below 960 CSS px the shell was replaced by a resize notice; the window opens at 940 px, and 200% zoom on a 1440-px display lands at 720 px | the shell reflows into a compact layout at every width; the notice is gone |
| **A11Y-7** | Medium | Applying | S9.6 / S9.7 | The Auto confirmation declared `aria-modal="true"` but neither moved focus nor trapped it | focus moves in, Tab is trapped, Esc closes, focus returns to the trigger |
| **A11Y-8** | Low | Tracker, Studio | S9.1 / S7.7 | `role="tab"` buttons with no `aria-controls`, no tabpanel and no arrow-key handling | `aria-controls` + `role="tabpanel"`, roving tabindex, ←/→/Home/End |

### A11Y-1 — Offer detail has a dangling label reference

`src/features/offer/index.tsx:65` names the page section with
`aria-labelledby="offer-title"`, but `Header` renders the `<h1>` without an id
(`src/features/offer/sections.tsx:316`). Screen readers announce an unnamed region.

**Fixed (CH14):** `id="offer-title"` on the `<h1>`.
**Evidence:** `e2e/a11y/screens.test.tsx` — "names the page region after the offer title".

### A11Y-2 — Tracker cards nest a button inside a button

`src/features/tracker/board/card.tsx` spread dnd-kit's `attributes` onto the
`<li>`, which gave it `role="button"` and `tabindex="0"`; the card's own open
control is a real `<button>` inside it. Two activation targets shared one name
family, and NVDA/VoiceOver reported a button inside a button (WCAG 4.1.2).

**Fixed (CH14):** the `<li>` is a plain list item again; dnd-kit's `attributes`
and `listeners` live on a native grip button ("Move {title}"), the open control
stays a button ("Open {title}"), and the Cmd/Ctrl + ←/→ handler stays on the card
so both controls drive it.
**Evidence:** `screens.test.tsx` — "keeps a card a list item with its own open and drag controls";
`keyboard.test.tsx` — the card still moves from its open button.

### A11Y-3 — Today nests a button inside a link

`src/features/today/matches/index.tsx` rendered
`<Link to="/inbox"><Button>Open Inbox</Button></Link>`. Invalid containment, and
several screen readers announced only one of the two controls.

**Fixed (CH14):** a single `<Link>` styled with `buttonVariants({ variant: "outline", size: "sm" })`.
**Evidence:** `screens.test.tsx` — "makes Open Inbox a single link control".

### A11Y-4 — `--fg-subtle` is painted on text

DESIGN.md §2.4 called `--fg-subtle` (4.11:1 light / 3.65:1 dark) **decorative
only**, but the app paints visible text with it in 32 files / 78 occurrences.
Normal-size text at those ratios failed WCAG AA.

**Fixed (CH14):** the token was re-tuned rather than the 32 files: light
`#756249`, dark `#A6907F`, both still warm. Measured ratios (light / dark):
bg 5.66 / 6.09 · surface 5.83 / 5.60 · surface-muted 4.99 / 5.08 ·
surface-sunken 4.66 / 6.23 · primary-soft (selected option) 4.81 / 4.52.
**Evidence:** `contrast.test.ts` — `PAIRS` now carries five `--fg-subtle` pairs in
both themes; DESIGN.md §2.4 records the values and ratios.

### A11Y-5 — "Strong78" is read as one word

`src/features/today/matches/index.tsx` put the Verdict word and the Rank number
in one span separated only by layout (`gap-1`). Text content was `Strong78`; a
screen reader said it as one token.

**Fixed (CH14):** a real `{" "}` between them; the accessibility tree reads "Strong 78".
**Evidence:** `screens.test.tsx` — the A11Y-5 case is now a normal passing test.

### A11Y-6 — 200% zoom replaces the app with a resize notice

`src/styles/index.css` hid `.shell` at `max-width: 959.98px` and showed
`.resize-floor` instead. `src-tauri/tauri.conf.json` allows the window down to
**940 px**, already inside that range. At 200% zoom a 1440-px window becomes
720 CSS px: content and function were lost, which EXPERIENCE §7 and NFR-15 forbid.

**Fixed (CH14):** the hide rule and the notice are gone; below 960 px the shell
reflows (56 px labelled icon rail, tighter topbar gutter), and the Studio's
inspector stacks under the editor. The sidebar keeps every destination reachable.
**Evidence:** `zoom.test.ts` — the shell is never `display: none`, the compact rule
covers 940 px and 720 CSS px, and the content column stays ≥ 320 px at both.
**Residual:** DESIGN.md §10.4 still describes a "< 960 floor screen" row; UX must
update that row to the compact layout (behaviour changed by this finding's fix).

### A11Y-7 — The Auto confirmation claims to be a modal but is not one

`src/features/settings/applying/rules/index.tsx` renders the "Turn on auto-apply?"
panel with `role="dialog" aria-modal="true"` inline in the page; focus stayed on
the trigger and Tab walked the whole screen.

**Fixed (CH14):** the confirmation is a real modal — focus moves to the acceptance
checkbox on open, Tab wraps inside it, Escape closes it, and focus returns to
"Enable Auto" (or to the screen title when Auto turned on and that button left).
**Evidence:** `screens.test.tsx` — "is a real modal: focus moves in, Tab is trapped,
Esc closes and restores focus".

### A11Y-8 — Tab lists are tabs in name only

Tracker (`src/features/tracker/index.tsx`) and Studio (`src/features/studio/index.tsx`)
rendered `role="tablist"` with `role="tab"` buttons but no `aria-controls`, no
`role="tabpanel"`, no roving tabindex and no ←/→ handling.

**Fixed (CH14):** each tab carries `aria-controls` and a roving `tabindex`, the
content region is a `role="tabpanel"` named by the selected tab, and
←/→/Home/End move the selection and the focus.
**Evidence:** `screens.test.tsx` — "exposes the view tabs as a complete tab pattern"
and the Studio equivalent.

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
- **Studio:** CV/cover tabs switch with Enter and with ←/→; the editor is
  reachable; Export is reachable; the apply bar's controls are all reachable.
- **Tracker:** Tab reaches a card's open control and its grip; the grip lifts
  with Space and moves with the arrows; Cmd/Ctrl + ←/→ moves a card; the
  confirmation for Applied is keyboard-operable; the stage collapse buttons are
  `aria-expanded`.
- **Applying:** Enable Auto opens the confirmation with focus on the checkbox,
  Space toggles it, Tab stays inside, Escape closes it and focus returns to
  Enable Auto; the confirm button is disabled until checked.
- **Kill Switch:** reachable in the header and via Cmd/Ctrl+Shift+K; Enter engages
  it; the engaged chip is announced (`role="status"`).

Expected: no focus trap (except the wizard and the Auto confirmation), no lost
focus after an action, a visible focus ring on every stop.

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

Expected after CH14: the compact layout (56 px icon rail, stacked Studio
inspector) keeps every surface usable at 200% zoom and at the 940-px window
minimum; no resize notice appears. This script confirms what `zoom.test.ts`
pins statically.

### E. Reduced motion (~10 min)

1. macOS: System Settings → Accessibility → Display → Reduce motion; Windows:
   Settings → Accessibility → Visual effects → Animation effects off; GNOME:
   Settings → Accessibility → Seeing → Reduce animation.
2. Open Today, start a run, open the Studio and drag a Tracker card.
3. Expected: no spinner rotation, no skeleton pulse, no smooth scroll — content
   appears in its final state. (dnd-kit's drag transform follows the pointer; it
   is direct manipulation, not animation.)
