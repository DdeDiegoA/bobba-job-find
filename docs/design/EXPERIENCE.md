# Boba Job Find — EXPERIENCE.md

> **Status:** final draft · **Owner:** ux · **Date:** 2026-10-04
> **Companions:** [`DESIGN.md`](./DESIGN.md) (how it looks) · [`voice-and-tone.md`](./voice-and-tone.md) (words) · [`brand.md`](./brand.md) · `docs/product/PRD.md` (binding vocabulary + FRs)
> Visual tokens are referenced as `{path.to.token}` and defined in `DESIGN.md`. **On conflict, the
> spines win over any mock or wireframe**; on vocabulary, the PRD glossary wins over this document.

---

## 1. Foundation

- **Form factor:** cross-platform **desktop** app (Tauri v2; Windows / macOS / Linux). Not a website.
- **UI system:** TypeScript + React; visual layer per `DESIGN.md` (Tailwind / shadcn-style tokens;
  final lib decided by architect — tokens ship in `DESIGN.md` §14).
- **Input:** mouse/keyboard primary; trackpad secondary. **No mobile surface.**
- **Offline:** the scoring core is deterministic and offline; network is needed only for live
  fetching, sending, and Agent CLI tasks. In **Limited mode** (no Agent CLI) search, scoring and the
  Tracker still work; AI-only actions are visibly disabled (FR-4).
- **Privacy:** local-only. No accounts, no billing, no backend. Secrets in the OS keychain; data
  encrypted at rest. The UI never claims a cloud sync.
- **Languages:** EN + ES, switchable at runtime; theme follows OS (light/dark) with override.

---

## 2. Information Architecture

Adopts the PRD §8 top level (UX confirmed); the ordering below is the sidebar order.

### 2.1 Primary navigation (sidebar)

```
Today            home               digest of the latest Search Run · needs-you items · Outbox
Inbox            inbox              ranked Offers + Offer detail
Studio           file-text          tailoring per Application (Document + Package)
Tracker          kanban-square      Stage kanban + Application detail
Profile          user-round         structured Profile, versions, Master CV
Connections      plug-2             Sources (by Risk Tier) + email accounts
Settings         sliders-horizontal engine, preferences, scoring, apply, privacy, appearance
```

Global affordances (topbar, always present): global search / command palette, **Run now**, run
status, and **Kill Switch** (only when Auto is on). Footer: Search Run status, theme toggle,
language toggle, Tapi. The **tray menu** (FR-42) mirrors: Open, Run now, Pause schedule, Kill Switch,
Quit.

### 2.2 Sitemap

```
Today
  ├─ latest Search Run digest (Verdict counts, new, needs-you, errors)
  ├─ Needs you — Status Proposals (quoted line, Approve/Dismiss) · Auto-bounced items · Needs-check
  ├─ Outbox (automatic sends in hold; Edit / Cancel)
  └─ links → Run report · Audit Log · Network activity
Inbox
  ├─ sections: Strong · Good · Moderate · Weak/Poor (collapsed) · Closing soon · Excluded
  ├─ filters: Source, work mode, language, salary-known, Verdict; sort by Rank/date/Fit/Quality
  ├─ Offer detail
  │     ├─ Gates with quoted evidence (+ Override)
  │     ├─ Explanation (Fit F1–F6 · Quality Q1–Q5 · matched/missing/evidence/confidence)
  │     ├─ duplicates & Sources · link to original · detected Apply Channels
  │     └─ actions: Save · Dismiss (reason) · Tailor · Open original
  └─ Excluded (VETO quotes + Override)
Studio
  ├─ current tailoring session (from an Offer)
  │     ├─ Document preview (CV + cover letter) · Template · EN/ES · diff
  │     ├─ claim traceability (every sentence → Profile item)
  │     ├─ reviewer findings · keyword coverage & declared gaps · ATS checks
  │     └─ Export PDF (ATS) / DOCX · Send
  └─ recent Application Packages (immutable once sent)
Tracker
  ├─ Stage kanban (Saved · Preparing · Applied · Interviewing · Offer · Hired · Rejected · No Response · Withdrawn)
  └─ Application detail: snapshot · Package sent · timeline · next action · notes
Profile
  ├─ sections (identity, experience, education, skills+evidence, projects, languages+CEFR, eligibility)
  ├─ Master CV import → review extracted fields with source excerpts (Confirm/Edit/Delete)
  ├─ EN/ES variants (draft missing) · version history / restore
  └─ re-score open Offers when the Profile changes
Connections
  ├─ Sources by Source Type (API · ATS Board · Feed · Manual · Custom · Session Board)
  │     risk badge per tile · health (OK / Degraded / Failing) · per-Source config
  ├─ Email (Gmail **send-only, read via IMAP/.eml/BYO** · Outlook/Graph read+send · IMAP-SMTP read+send)
  └─ custom Source: RSS/feed · ATS board (MVP); v1.1: generic careers page / SDK / LATAM boards.
     **Session Board is MVP** for built-in LinkedIn + Indeed (opt-in, ToS risk)
Settings
  ├─ AI engine (CLI, model, run budget)
  ├─ Profile & Preferences
  ├─ Sources · Email
  ├─ Apply (Review/Auto · Auto Rules editor per Source × Apply Channel · Outbox hold · caps ·
  │        dry-run results · per-Source ToS opt-in with typed confirmation)
  ├─ Schedule & notifications
  ├─ Scoring weights · Language & theme · Privacy & data · About
  └─ Appearance (theme, density, reduce motion)
Small views (read-only, reachable from Today/Settings)
  ├─ Run report (FR-46) · Audit Log (FR-89) · Network activity (NFR-1) · Source health (FR-31)
Onboarding (first-run modal wizard over the shell)
Tray menu + OS notifications + global Kill Switch shortcut (outside the window)
```

### 2.3 Content model (shared vocabulary — PRD §3)

- **Offer** — one normalised posting (title, company, locations, work mode, salary, posted/deadline,
  description, language, source URLs, Apply Channel hints). A **Duplicate Cluster** groups the same
  job across Sources; the Inbox shows the canonical Offer.
- **Fit Score / Quality Score / Rank Score** (0–100) · **Verdict** (band of Rank) · **Confidence**
  (High/Medium/Low) · **Explanation** (factors + narration) · **Gate** (PASS/FLAG/VETO + quote).
- **Application** with a **Stage** and an **Application Package** (Documents + answers; immutable
  once sent).
- **Document** (CV/cover letter) in a **Template** and a language.
- **Source** (Source Type + Risk Tier) · **Connection**.
- **Auto Rule** · **Outbox** · **Kill Switch** · **Audit Log** · **Status Proposal**.

Every screen is a doorway into the **Offer**: from Today, the Inbox, the Tracker, a notification, or
search. Offer detail is the hub; the **Studio** is the terminal action.

---

## 3. Voice and Tone

Full copy in [`voice-and-tone.md`](./voice-and-tone.md). Behavioral summary: warm, honest, calm;
encourage effort not outcomes; **name the gap then the path**; the user decides; local-first said out
loud; errors calm and specific; rejections kind and never nudging; generated Documents use the user's
voice, not Boba's.

---

## 4. Component Patterns (behavioral)

Visual specs live in `DESIGN.md` §10. Behavioral rules here.

- **OfferCard** — clicking opens Offer detail; quick actions (**Save**, **Dismiss**, **Tailor**) do
  not. Save creates an Application in **Saved**; Dismiss removes it from the Inbox with an optional
  reason and offers **Undo** (6s). Selected card is the keyboard focus cursor for `J/K`.
- **DualScore** — read-only. Fit/Quality rings show numbers; the **Verdict** chip shows the band of
  the Rank Score. Clicking opens the Explanation and scrolls to the group. Weights change only in
  Settings.
- **Explanation panel** — Gates at top (VETO shows the quote + **Override**); factor rows expand to
  matched/missing/evidence. The narration is labeled and never edits numbers. Read-only truth.
- **Kanban** — drag or `Cmd/Ctrl+←/→` to change Stage. Moves are optimistic with Undo. Moving to
  **Applied** requires submission evidence (FR-88); without it the card is **Needs check**. Every
  change is logged (FR-93). Terminal Stages are collapsible.
- **Studio** — generation runs draft → fresh-context review → revise (FR-72). **Claim traceability**
  flags any sentence with no linked Profile item (FR-73). A failed no-fabrication or ATS check blocks
  *Ready/Send* in Auto and warns in Review (FR-74, FR-77). A sent Package is immutable.
- **Wizard** — forward only when a step is valid; **Back** never loses input; safe steps are
  skippable with a visible "Skip for now"; resume banner on reopening. Auto cannot be enabled here.
- **ConnectionTile** — Connect/Manage opens auth (OAuth in the system browser, IMAP/SMTP form, or an
  opt-in Session Board with a ToS warning + typed confirmation). Disable ≠ disconnect; disconnect
  requires confirm and purges tokens (FR-39).
- **Outbox** — items are **editable and cancellable** during the hold; a notification fires when one
  enters; the Kill Switch empties it into the review queue.
- **Profile** — every extracted field is reviewed with its **source excerpt** before it enters the
  Profile (FR-5). Confirmed changes create a version (FR-13) and mark open Offer scores stale (FR-14).
  A missing EN/ES variant can be drafted, but must be confirmed before it can be sent (FR-12).
- **Status Proposal** — an email-derived Stage change is *proposed*, never applied silently; the card
  shows the quoted line and confidence, with **Approve** / Dismiss. Low-risk types auto-approve only
  if the user opted in (FR-103).
- **Auto Rule editor** — one rule per **Source × Apply Channel**; enabling Auto is a dialog +
  checkbox. A **Prohibited-automation** Source needs the typed **ToS confirmation**; the **ToS risk**
  badge then persists on the tile and on every rule targeting it (FR-91). New rules show a **dry run**
  first (FR-84).
- **Red-flag / fraud badges** — a likely-ghost badge lowers Q4 Legitimacy; a recruitment-fraud badge
  **blocks Auto** for that Offer (FR-60). Both are icon + text, never color alone.
- **Small views** (Run report, Audit Log, Network activity, Source health) — read-only, sortable,
  keyboard-navigable, with an empty state and CSV export where applicable.
- **Limited mode** — with no Agent CLI, AI-only actions (Profile extraction, Tailoring, email
  classification, narration) render disabled with a "Connect an agent" CTA; search, scoring and the
  Tracker keep working (FR-4).
- **Tray panel** — status-first; hides on focus loss; keyboard: arrows + Enter.

---

## 5. State Patterns

| State | Behavior |
|---|---|
| **Loading** | Skeletons shaped like the final content (no spinners for content regions). Region `aria-busy`. Re-runs keep the previous list visible and dimmed ("stale"), never blank. |
| **Empty (true)** | Tapi + one headline + one CTA. Only when genuinely nothing. |
| **Empty (filtered)** | No mascot. "No Offers for {filters}. Clear filters." |
| **Error** | Inline `InlineAlert` near the failed region + recovery action. Partial failures never blank the page. Background failures → non-blocking toast. |
| **Success** | Quiet confirmation + next step. Reversible actions include Undo. |
| **Partial** | Some Sources failed during a run → the run still succeeds; a banner lists failures with Retry. |
| **Offline** | Non-blocking banner; search/tailoring need a connection; local data fully available. |
| **Limited mode** | No Agent CLI: AI-only actions disabled with a "Connect an agent" CTA; deterministic search/scoring/Tracker still work. |
| **Unsupported for this task (P0-2)** | Untrusted-content tasks run **only** on a CLI with verified `tool_lockdown`. If the chosen CLI lacks it, the task shows a `TaskUnsupportedNotice` (plain reason + **Change CLI**) and **cannot be overridden** — never a clickable control that fails later. In Limited mode every such task is unsupported. |
| **Locked (P1-5)** | Passphrase-derived key not unlocked this login session: persistent **LockedBanner** + **UnlockDialog** on gated actions. A scheduled run while locked is recorded **skipped** and notifies "Boba is locked…" — never a silent success. Unlock is once per login session, cached in zeroized memory with an idle timeout. |
| **Low Confidence** | Verdict shown as **Lead** (dashed chip, "worth checking"), never a hard band. |
| **Stale** | "Last updated {relative}" chip; manual **Re-score now** after a Profile change (FR-14). |
| **Running (Search Run)** | Topbar status + tray; actions depending on fresh results note "results from {time}". |
| **Needs check** | A send without confirmation evidence; card marked, opens the Application to verify (FR-88). |
| **Auto on / off / Kill Switch** | Quiet persistent status chip; Kill Switch is never a red alarm. |
| **Density / Theme / i18n** | Tokens only; no layout or content change. |

---

## 6. Interaction Primitives

- **Selection model:** `J/K` focus cursor in lists; `Space` toggles multi-select; `Shift` range.
  Focus is visible and restored after navigation.
- **Undo everywhere reversible/destructive:** Save, Dismiss, Stage move, Outbox cancel, auto-apply
  pause. Undo lives in the toast and on the last action.
- **Optimistic actions** with rollback + toast on failure (save/dismiss/move).
- **Confirmation ladder:** reversible → no dialog (toast + Undo); external side-effect (Send, enable
  Auto, Session Board opt-in) → confirm dialog (+ typed confirmation for ToS-risk); destructive/data
  (delete data, disconnect) → confirm + explicit acknowledgment.
- **Progressive disclosure:** scores visible; Explanation on demand; advanced settings behind a
  disclosure; the wizard surfaces only what each step needs.
- **Run feedback:** the Search Run is observable (which Source, how far) but never blocks the UI.
- **Kill Switch** is reachable in 2 s from anywhere: tray, app header, `Cmd/Ctrl+Shift+K`.
- **Command palette** is the fast path to any page, Offer, or action; it also runs searches.
- **Notifications** mirror the in-app digest; clicking deep-links to the Offer/Application.

---

## 7. Accessibility Floor

Behavioral commitments (visual contrast lives in `DESIGN.md` §2.4/§12; WCAG 2.2 AA per NFR-15):

- Every flow is completable **by keyboard alone**.
- Focus-visible ring on every interactive element; focus trapped in modals/palette, restored on close.
- Scores, Verdicts, Gates, Stages and statuses are announced as **text**, never as color.
- Live regions announce run progress and toasts; errors use `role=alert`.
- `prefers-reduced-motion` and the in-app setting remove movement, spring, and animated meters;
  content appears in its final state.
- Zoom to 200% without loss of content or function; no horizontal scroll on content columns.
- Language and theme changes are announced and persisted.

---

## 8. Key Flows

These map to the PRD journeys UJ-1…UJ-6. Protagonist: **Diego**, a LATAM backend developer applying
for global remote roles, on macOS, ES/EN.

### Flow A — UJ-1: Diego sets up Boba on a Sunday evening (climax: a ranked list in 12 minutes)

1. **Language & theme.** He picks Español; the theme follows the OS.
2. **Agent CLI.** The wizard detects **Claude Code** and **OpenCode**; he picks Claude Code with
   Sonnet and the test call succeeds (latency shown). *If none were found:* it explains what an Agent
   CLI is, links install guides, and offers **Limited mode**.
3. **Profile import.** He drops his English Master CV PDF. Boba extracts experience, skills and
   languages; every field is shown with its source excerpt.
4. **Profile gaps.** Three questions the CV couldn't answer: work authorisation, English CEFR, and
   his **Salary Floor** (USD).
5. **Preferences.** Target roles, "remote / LATAM ±3h", a $4,000/month floor.
6. **Sources.** Recommended Safe Sources pre-selected (remote feeds, ATS boards, Adzuna); Risk Tiers
   shown; he keeps them.
7. **Email.** Connects Gmail **send-only** (bundled public client); reply reading stays off — later via
   Outlook/Graph, IMAP/.eml import, or his own OAuth client ID. Apply Mode stays on **Review**
   (Auto cannot be enabled in the wizard).
8. **Recovery backup** *(P0-1, only on unstable-signing builds).* When the build's signing can't
   survive an update, he saves a one-time **recovery key** + backup file before anything runs — the
   step is hidden entirely on stably-signed builds.
9. **First run.** Finishing starts a **Search Run** immediately.
   → **Climax:** 41 new Offers, 6 **Strong**. He can already see Fit/Quality, Verdicts and the *why*.
   Target: install → first ranked list ≤ 15 min (SM-1).

### Flow B — UJ-2: Diego triages over coffee (climax: all 14 decided in under 10 minutes)

1. **08:00 digest.** Tray runs; notification: *"3 Strong, 5 Good today."* He opens the Inbox.
2. **Top Offer.** Fit 84, Quality 71, Verdict **Strong**, Confidence High. Detail: "You match 7/8
   must-haves; missing Kubernetes. Salary $5–6k is above your floor. Posted 4 days ago on the
   employer's Greenhouse board."
3. **Closing soon.** A role flagged *Closing soon*; he reads the Gate quote *"Must be authorized to
   work in the US"* and **Dismisses** it (reason: not eligible).
4. **Keyboard triage.** `J/K` to move, `S` to Save two Offers, `X` to Dismiss others.
   → **Climax:** 14 Offers decided in under 10 minutes (SM-2).

### Flow C — UJ-3: Diego tailors and sends (climax: a Spanish application sent from Gmail)

1. On a saved **Spanish** Offer from a Madrid company, he clicks **Tailor** → the **Studio**.
2. Boba detects Spanish and drafts a 2-page CV + 1-page letter using the **Clean** Template; a
   fresh-context reviewer (FR-72) critiques, then it revises.
3. The Studio shows **claim traceability** (every bullet linked to its Profile source), keyword
   coverage 12/14 with "Terraform" and "GCP" listed as **declared gaps** (never invented), and green
   checks "ATS text layer OK", "2 pages, no orphans".
4. He edits one sentence and clicks **Send**. The posting said "send your CV to jobs@…", so Boba
   sends the email from his Gmail with both PDFs attached.
   → **Climax:** the Application moves to **Applied** with the exact Package archived; reply watching
   begins.

### Flow D — UJ-4: Diego turns on bounded Auto (climax: "2 sent automatically, 1 needs you")

1. **Enable Auto.** Dialog + explicit checkbox. He scopes Auto Rules to *Email Apply* and *ATS Form*
   on remote Sources: Rank ≥ 80, no FLAGs, Confidence ≥ Medium, 5/day, 1/company, 30-min Outbox hold.
2. **Dry run.** The first 3 matches run as a **dry run**; Boba shows what it *would* send; he reviews
   and activates.
3. **Overnight / morning.** Two applications go out; each waits in the **Outbox** first.
4. **CAPTCHA.** An ATS form hits a CAPTCHA — Boba stops and moves it to his review queue (FR-87/88).
   → **Climax:** digest reads *"2 sent automatically, 1 needs you."*
   **Edge case:** during a bad week he hits the tray **Kill Switch**; all automatic sending stops
   within 2 s and Outbox items move to review.

### Flow E — UJ-5 + rejection recovery (climax: he takes a break instead of spiraling)

1. **Email parsed.** A reply arrives; Boba links it to the Application and posts a **Status
   Proposal**: *"We'd like to schedule a call…"* → "Applied → Interviewing", quoting the line.
2. **Approve.** He accepts with one click; the Tracker card moves to **Interviewing**.
3. **A rejection arrives** the same week. The proposal reads: *"Update on {company}: they've moved
   forward with other candidates. That happens — and it isn't a verdict on you."* No pop-up; a muted
   card.
4. **Choices:** save the company, find similar Offers, take a break.
5. **Cluster.** Two more rejections → the app offers: *"A few nos in a row is normal. Want to pause
   the schedule for today?"*
   → **Climax:** he accepts the pause; the app never nags him to apply more.

### Flow F — UJ-6: Diego adds a company he loves

1. He pastes `https://boards.greenhouse.io/acme` as a custom **Source**.
2. Boba recognises a Greenhouse **ATS Board**, runs a test fetch returning 23 jobs, and adds it to
   the next Search Run.

---

## 9. Key Screens (low-fi wireframes)

> Structural, not pixel specs. Real spacing/type/color come from `DESIGN.md`.
> `[ ]` = button · `( )` = chip · `▾` = menu · `◉` = ring · `·` = meta separator · `⚠` = flag.

### 9.1 Onboarding wizard (FR-1…FR-10 order)

```
┌──────────────────────────────────────────────────────────────────────────────┐
│  (boba)                        ●─●─○─○─○─○─○─○─○   Step 3 of 9               │
├──────────────────────────────────────────────────────────────────────────────┤
│   [ Tapi pointing ]    Import your Master CV                                  │
│                        Drop a PDF, DOCX or LinkedIn export. We'll read it     │
│                        and fill your Profile. Nothing saves until you         │
│                        confirm each field.                                    │
│                        ┌──────────────────────────────────┐                   │
│                        │  ⬆  Drop file here, or browse     │                   │
│                        └──────────────────────────────────┘                   │
│                        ⓘ Everything stays on your computer.                   │
│                        Detected: María… no — Diego Arenas · 8 skills · EN/ES  │
│                        [ Review extracted fields ]                            │
├──────────────────────────────────────────────────────────────────────────────┤
│   [ Skip for now ]                                      [ Back ] [ Next ]     │
└──────────────────────────────────────────────────────────────────────────────┘
```

Steps: 1 Language & theme · 2 Agent CLI (detect/test/Limited mode) · 3 Profile import ·
4 Profile gaps · 5 Preferences · 6 Sources · 7 Email (optional) · 8 Apply Mode & schedule ·
9 **Recovery backup** (P0-1 — only on builds whose signing cannot survive an update) · 10 First run.
**Next** disabled with inline reason until valid; **Back** never loses input.

**Recovery backup (P0-1).** On unsigned/ad-hoc/self-signed builds whose Keychain access may not
survive an update, a **mandatory** step runs before the first run: the user saves a one-time
**recovery key** and a backup file, so a future update can never lock them out of their encrypted
data. The step is **hidden on builds whose signing is stable** (no friction when it isn't needed) and,
when shown, **cannot be skipped** — **Next** stays disabled until the key is acknowledged.

```
Recovery backup — step 9 (unstable-signing builds only)
┌───────────────────────────────────────────────────────────────┐
│   [ Tapi sealing a cup ]    Save your recovery key            │
│   This build isn't signed in a way that survives every update.│
│   If an update locks the app out of your encrypted data, this │
│   key is how you get back in. Keep it safe — it's the only    │
│   copy; Boba can't recover it for you.                        │
│   Recovery key   B0BA-4F7K-9Q2M-XT8R-61ZD   [ Copy ] [ Print ] │
│   [ Save backup file… ]   Boba-backup-2026-10-05.boba         │
│   ☑  I've saved my recovery key somewhere safe                │
├───────────────────────────────────────────────────────────────┤
│                                              [ Back ] [ Next ]│
└───────────────────────────────────────────────────────────────┘
```

- Explains the *why* plainly (updates + local encryption), never as scary jargon.
- Recovery key shown **once**, with **Copy / Print / Save file**; no "later" deferral on this build.
- If the upgrade acceptance test later fails after an update, the app opens a **recovery screen**
  (paste recovery key → restore from the backup), not a dead end. Reviewed in Settings ▸ Privacy.

### 9.2 Today

```
┌──────────────┬───────────────────────────────────────────────────────────────┐
│ (boba)       │ Today                          [ 🔎 ]   [ Run now ]   [Kill ⏻] │
│              │ Last run 06:12 · 6 Sources · 12 new · Auto on                 │
│ ● Today      ├───────────────────────────────────────────────────────────────┤
│ ○ Inbox      │  ⚠ 1 Source failed (Remotive) — everything else ran. [Retry]  │
│ ○ Studio     │  ┌─────────────────────────────────────────────────────────┐  │
│ ○ Tracker    │  │ 3 Strong · 5 Good · 4 Moderate          [ Open Inbox ]   │  │
│ ○ Profile    │  └─────────────────────────────────────────────────────────┘  │
│ ○ Connections│  Needs you (2)                                                │
│ ○ Settings   │  ┌─────────────────────────────────────────────────────────┐  │
│              │  │ API Engineer · Cirrus   Fit 88 Quality 80  ( Strong )    │  │
│ ── footer ── │  │ ATS Form hit a CAPTCHA — needs your review   [ Review ]  │  │
│ [Tapi] 06:12 │  └─────────────────────────────────────────────────────────┘  │
│ [☾] [ES/EN]  │  Outbox (2, sending in 24 min)                                │
│              │  ┌─────────────────────────────────────────────────────────┐  │
│              │  │ Data Engineer · Tapi Tea Co   [Review now] [Cancel]      │  │
│              │  │ Sr Backend · Nimbus           [Review now] [Cancel]      │  │
│              │  └─────────────────────────────────────────────────────────┘  │
└──────────────┴───────────────────────────────────────────────────────────────┘
```

### 9.3 Inbox

```
┌──────────────┬───────────────────────────────────────────────────────────────┐
│ (boba)       │ Inbox                     [ 🔎 search ]   sort: Rank ▾  ▢    │
│ ● Today      ├───────────────────────────────────────────────────────────────┤
│ ● Inbox      │ ( Strong ) ( Remote ) ( salary known ) ( Closing soon )       │
│ ○ Studio     │ ▸ Strong                                                      │
│ ○ Tracker    │ ┌───────────────────────────────────────────────────────────┐ │
│ ○ Profile    │ │ (C) Senior Backend Eng   ◉82 ◉71  [● Strong · 78]  conf Med │ │
│ ○ Connections│ │     Nimbus · Remote (EU)   €65–80k · 2d · Greenhouse      │ │
│ ○ Settings   │ │     ✓ dbt ✓ Airflow ✗ Snowflake      [Save][Dismiss][Tailor]│ │
│              │ └───────────────────────────────────────────────────────────┘ │
│              │ ▸ Good                                                        │
│              │ ┌───────────────────────────────────────────────────────────┐ │
│              │ │ (C) Data Engineer        ◉74 ◉84  [◗ Good · 78]   conf High│ │
│              │ │     Tapi Tea Co · Remote   $90–110k · 1d · RSS            │ │
│              │ │     ✓ Python ✓ SQL ✗ Kafka           [Save][Dismiss][Tailor]│ │
│              │ └───────────────────────────────────────────────────────────┘ │
│              │ ▸ Moderate   ▸ Weak / Poor (4, collapsed)                     │
│              │ ▸ Closing soon   ▸ 4 Excluded — review                        │
└──────────────┴───────────────────────────────────────────────────────────────┘
```

- Rows: `◉Fit ◉Quality` rings + **Verdict chip** (band of Rank) + Confidence. Never color alone.
- Actions left→right: **Save** (→ Stage Saved) · **Dismiss** (optional reason, undo) · **Tailor**.

### 9.4 Offer detail (Inbox, with Explanation inspector)

```
┌──────────────┬────────────────────────────────────────────┬──────────────────┐
│ (boba)       │ Senior Backend Engineer                    │  EXPLANATION     │
│ ● Inbox      │ Nimbus · Remote (EU) · €65–80k · 2 days ago│  Gates           │
│              │ (C) (Remote) (Greenhouse) (⚠ Closing soon) │  ✓ PASS location  │
│              ├────────────────────────────────────────────┤  ⚠ FLAG seniority │
│              │  ┌────────────┐  ┌────────────┐   ┌───────┐ │   "Senior role…"  │
│              │  │    Fit     │  │  Quality   │   │ Rank  │ │  Fit ▸           │
│              │  │   ◉ 82     │  │   ◉ 71     │   │ ●78   │ │   F1 Skills 88 ● │
│              │  │  (Encaje)  │  │ (Calidad)  │   │Strong │ │   F2 Exp    58 ◐ │
│              │  └────────────┘  └────────────┘   └───────┘ │   … ▸            │
│              │  Confidence: Medium ▏▎▍▎▏                  │  Quality ▸       │
│              │  Narration: "You match 7/8 must-haves;      │   Q1 Comp   92 ● │
│              │  missing Kubernetes. Salary is above your   │   Q2 Company35 ○ │
│              │  floor…"                                    │   … ▸            │
│              │  Matched ✓ dbt ✓ Airflow ✗ Snowflake        │  Weights 65/35 ⚙ │
│              │  Duplicates: 2 more Sources                 │                  │
│              │  Apply Channels: Email Apply · ATS Form     │                  │
│              │  [ Tailor ] [ Save ] [ Dismiss ▾ ] [Open original]            │
└──────────────┴────────────────────────────────────────────┴──────────────────┘
```

- Inspector inline ≥1280; overlay drawer 960–1279. Evidence quotes always reachable.
- **Tailor** is the single primary action.

### 9.5 Studio (tailoring per Application)

```
┌──────────────┬───────────────────────────────┬───────────────────────────────┐
│ (boba)       │ CV · Diego Arenas             │ Reviewer findings             │
│ ● Studio     │ [ Classic ▾ ][ Clean ▾] (ES ▾)│ ✓ Reordered to lead with dbt  │
│              │ ┌───────────────────────────┐ │ ✓ Added Snowflake (learning)  │
│              │ │  paper preview (A4)       │ │ ⚠ 2 declared gaps: Terraform, │
│              │ │  ~ highlighted changes ~  │ │   GCP — not added (no source) │
│              │ │                           │ │                               │
│              │ │  Experience               │ │ Claim traceability            │
│              │ │   ▪ ▪ ▪  ← linked to      │ │ ● every bullet → Profile item │
│              │ │      Profile items        │ │ ⚠ 1 sentence unlinked [Fix]   │
│              │ └───────────────────────────┘ │                               │
│              │  Coverage 12/14 · pages 2/2   │ ATS: ✓ text layer · ✓ no (cid)│
│              │  no orphans · [ diff ▾ ]      │ Keyword check ✓               │
├──────────────┴───────────────────────────────┴───────────────────────────────┤
│  ( ) Diff: previous ↔ current   [ Export PDF ] [ DOCX ]        [ Send ]       │
└──────────────────────────────────────────────────────────────────────────────┘
```

- No-fabrication (FR-74) and ATS (FR-77) checks block **Send** in Auto; warn in Review.
- Export buttons are always available once ready; the Package is archived on Send.

### 9.6 Tracker (kanban, PRD Stages)

```
┌──────────────┬───────────────────────────────────────────────────────────────┐
│ (boba)       │ Tracker                     [ ▤ Kanban | ☰ List ]            │
│ ● Tracker    ├───────┬───────┬────────┬───────┬───────┬── Hired▾ ───┬───────┤
│              │Saved 5│Prep. 2│Applied │Interv.│Offer 1│Rejected 3   │NoResp.│
│              │───────┼───────┼────────┼───────┼───────┼─────────────┼───────│
│              │┌─────┐│┌─────┐│┌──────┐│┌─────┐│┌─────┐│┌─────┐      │┌─────┐│
│              ││Back │││Data │││API   │││ML   │││Sr FE│││SRE  │      ││…    ││
│              ││end  │││Eng  │││Eng   │││Eng  │││     │││(n)  │      ││     ││
│              ││ ●78 │││ ◗74 │││ ●88  │││ ●91 │││ ●95 ││└─────┘      │└─────┘│
│              │└─────┘│└─────┘│└──────┘│└─────┘│└─────┘│             │       │
│              │ drag / [Cmd+←→] · every change logged (FR-93)                 │
└──────────────┴───────┴───────┴────────┴───────┴───────┴─────────────┴───────┘
```

- Terminal Stages collapse (Hired / Rejected / No Response / Withdrawn). List mode <900px.
- Moving to **Applied** needs submission evidence (FR-88) or the card is **Needs check**.

### 9.7 Connections (Sources & email)

```
┌──────────────┬───────────────────────────────────────────────────────────────┐
│ (boba)       │ Sources & Connections                          [ + Custom ]   │
│ ● Connections│ Sources                                                       │
│              │ ┌───────────────┐ ┌───────────────┐ ┌───────────────┐          │
│              │ │ Greenhouse    │ │ Remotive      │ │ LinkedIn      │          │
│              │ │ ATS Board     │ │ Feed          │ │ Session Board │          │
│              │ │ (Safe)        │ │ (Safe)        │ │ (Prohibited)  │          │
│              │ │ ● OK 06:12    │ │ ⚠ Failing 2×  │ │ ○ Opted out   │          │
│              │ │ [Manage ▸]    │ │ [Manage ▸]    │ │ [Enable ⚠]    │          │
│              │ └───────────────┘ └───────────────┘ └───────────────┘          │
│              │ Email                                                         │
│              │ ┌───────────────┐ ┌───────────────┐ ┌───────────────┐          │
│              │ │ Gmail         │ │ Outlook/Graph │ │ IMAP/SMTP     │          │
│              │ │ OAuth·send    │ │ OAuth·read+   │ │ ○ Not set up  │          │
│              │ │ ✓ Connected   │ │   send ✓      │ │ [Set up]      │          │
│              │ │ [Manage ▸]    │ │ [Manage ▸]    │ │               │          │
│              │ └───────────────┘ └───────────────┘ └───────────────┘          │
│              │ Send-only by default. Read replies via Outlook/Graph,         │
│              │ IMAP/.eml import, or "Enable read (advanced)" with your own   │
│              │ OAuth client ID — the bundled client cannot read Gmail.       │
└──────────────┴───────────────────────────────────────────────────────────────┘
```

- Every tile shows **Source Type** (API · ATS Board · Feed · Manual · Custom · Session Board) and
  **Risk Tier** (Safe · Opt-in · Prohibited-automation). Enabling an opt-in tier requires the
  warning; Session Board **Auto** needs the second opt-in + typed confirmation (FR-91).
- **Email tiles show capability per provider** (see DESIGN "Sources & Connections grid"): Gmail =
  **send-only** by default (bundled public client); **read** only via BYO OAuth client ID or
  IMAP/.eml import. Outlook/Graph and IMAP/SMTP = **read+send**. Never label Gmail "read+send".

### 9.8 Settings

```
┌──────────────┬───────────────────────────────────────────────────────────────┐
│ (boba)       │ Settings                                                      │
│ ● Settings   │ ┌ Engine ┐ Profile ┐ Preferences ┐ Scoring ┐ Apply ┐ Schedule ┐│
│              │                                                               │
│              │ Apply                                                         │
│              │ Apply Mode     ( Review ) ( Auto )                            │
│              │ Auto Rules     [ + New rule ]                                 │
│              │   • Email Apply · remote   Rank≥80 Fit≥80 Conf≥Med 5/day      │
│              │   • ATS Form · Safe        Rank≥85           3/day  [Edit]    │
│              │ Outbox hold    [ 30 min ]     Caps: 5/day · 1/company/30d     │
│              │ Kill Switch    ● Ready        [ View Audit Log ]              │
│              │                                                               │
│              │ Scoring        Fit weight ──●──── 65 %                        │
│              │                Quality   ────●── 35 %                        │
│              │                Calibration: 0 outcomes — need 20 to advise.  │
│              │ Privacy        🔒 Encrypted · local keychain                  │
│              │                [ Export backup ]  [ Delete all data ]         │
└──────────────┴───────────────────────────────────────────────────────────────┘
```

- High-stakes sections (Apply, Privacy) use the confirmation ladder; enabling Auto is a dialog +
  checkbox. Calibration suggestions are opt-in and suppressed under a small sample (FR-62).

### 9.9 Tray menu & Outbox

```
        ┌───────────────────────────────────────────┐
        │  boba                          [ ☾ ] [ ⚙ ] │
        │  Last run 06:12 · 12 new · Auto on        │
        │  3 Strong · 5 Good · 4 Moderate           │
        │  Outbox (2, hold 24 min)                  │
        │  ┌───────────────────────────────────────┐ │
        │  │ API Engineer · Cirrus        ●88      │ │
        │  │ Data Engineer · Tapi Tea Co  ◗74      │ │
        │  └───────────────────────────────────────┘ │
        │  [ Open Boba ] [ Run now ] [ Pause ]      │
        │  [ ⏻ Kill Switch ]                        │
        └───────────────────────────────────────────┘
```

---

### 9.10 Profile

```
┌──────────────┬───────────────────────────────────────────────────────────────┐
│ (boba)       │ Profile                          ( EN │ ES )   v7 · History   │
│ ● Profile    ├──────────────┬────────────────────────────────────────────────┤
│              │ sections     │ Experience                                     │
│              │ Identity     │ ┌────────────────────────────────────────────┐ │
│              │ Experience ◄ │ │ Senior Backend Eng · Nimbus · 2022–now     │ │
│              │ Education    │ │ • Led the dbt/Airflow migration            │ │
│              │ Skills       │ │   evidence: role “Nimbus” [ confirm ✓ ]    │ │
│              │ Projects     │ └────────────────────────────────────────────┘ │
│              │ Languages    │ ⚠ ES variant missing here        [ Draft ES ]  │
│              │ Eligibility  │ [ + Add bullet ]   edits create a new version  │
│              └──────────────┴────────────────────────────────────────────────┘
```

Master CV import shows each extracted field with its **source excerpt** and Confirm/Edit/Delete;
nothing enters the Profile until confirmed (FR-5). Confirmed edits create a version (FR-13) and mark
open Offer scores stale (FR-14).

### 9.11 Inbox · Excluded & Closing soon

```
┌──────────────┬───────────────────────────────────────────────────────────────┐
│ (boba)       │ Inbox ▸ Excluded (4)                                          │
│ ● Inbox      ├───────────────────────────────────────────────────────────────┤
│              │ ┌───────────────────────────────────────────────────────────┐ │
│              │ │ (C) Site Reliability Eng · Orbit                          │ │
│              │ │ ⊘ VETO — work eligibility                                 │ │
│              │ │ “Must be authorized to work in the US.” — quoted          │ │
│              │ │        [ Override and score ]   [ Open original ]         │ │
│              │ └───────────────────────────────────────────────────────────┘ │
│              │ Closing soon                                                  │
│              │ ┌───────────────────────────────────────────────────────────┐ │
│              │ │ (C) Platform Eng · Cirrus                                 │ │
│              │ │ ⚑ Likely ghost (posted 120d)   ⛔ Recruitment-fraud risk  │ │
│              │ │    → Auto blocked for this Offer                          │ │
│              │ └───────────────────────────────────────────────────────────┘ │
└──────────────┴───────────────────────────────────────────────────────────────┘
```

Verdicts always show **text + icon** (NFR-15). A fraud badge blocks Auto (FR-60); a VETO is shown with
its quote and an override (FR-54), never silently deleted.

### 9.12 Settings ▸ Apply (Auto Rules · dry run · ToS opt-in)

```
┌──────────────┬───────────────────────────────────────────────────────────────┐
│ (boba)       │ Settings ▸ Apply                                              │
│ ● Settings   │ Apply Mode   ( Review ) ( Auto )                              │
│              │ Auto Rules                                    [ + New rule ]   │
│              │ ┌───────────────────────────────────────────────────────────┐ │
│              │ │ Email Apply × remote Sources          [on]                │ │
│              │ │ Rank≥80 · Fit≥80 · Conf≥Med · no FLAGs · 5/day · 1/co/30d │ │
│              │ ├───────────────────────────────────────────────────────────┤ │
│              │ │ ATS Form × Safe Sources               [on]                │ │
│              │ ├───────────────────────────────────────────────────────────┤ │
│              │ │ Session Board × LinkedIn   ⚠ ToS risk  [off]              │ │
│              │ │   opt-in needs typing “LinkedIn”        [ Set up ]        │ │
│              │ └───────────────────────────────────────────────────────────┘ │
│              │ Dry run — “Email Apply × remote” (first 3)                    │
│              │   • Data Eng · Tapi Tea  — would send · Rank 84 · conf High   │
│              │   • API Eng · Cirrus     — would SKIP · FLAG (seniority)      │
│              │   [ Activate rule ]                                           │
│              │ Outbox hold [ 30 min ]   Caps 5/day · 1/company  [ Audit Log ]│
└──────────────┴───────────────────────────────────────────────────────────────┘
```

Enabling Auto shows a confirmation dialog + checkbox. A **Prohibited-automation** Source requires the
typed **ToS confirmation** (type the Source name), recorded in the Audit Log (FR-91), and shows the
persistent **ToS risk** badge.

### 9.13 Small views & diagnostics

```
┌──────────────┬───────────────────────────────────────────────────────────────┐
│ (boba)       │ Run report ·  Audit Log ·  Network activity          (tabs)   │
│ ● Today      ├───────────────────────────────────────────────────────────────┤
│              │ Run report — Sun 06:12 · 3m 41s                               │
│              │  Sources 6 · fetched 214 · new 12 · dup 188 · vetoed 4 ·       │
│              │  ranked 8 · agent tasks 26/30 · errors 1                      │
│              │  ⚠ Remotive: HTTP 503 (2 in a row)                            │
│              ├───────────────────────────────────────────────────────────────┤
│              │ Audit Log                                       [ Export CSV ] │
│              │  time       event          target            reason           │
│              │  06:14:02   Send (Auto)    Data Eng·Tapi Tea  rule email×rem  │
│              │  06:13:55   Skip (Auto)    Jr FE·Nova         Rank 61 < 80    │
│              │  06:10:11   Stage change   API Eng·Cirrus     → Applied       │
│              ├───────────────────────────────────────────────────────────────┤
│              │ Network activity                                              │
│              │  greenhouse.io · remotive.com · gmail.googleapis.com ·        │
│              │  api.anthropic.com (Agent CLI) · no telemetry                 │
└──────────────┴───────────────────────────────────────────────────────────────┘
```

### 9.14 Limited mode (no Agent CLI)

```
┌───────────────────────────────────────────────────────────────┐
│ ⓘ AI features are off — no Agent CLI connected.               │
│   Search, scoring and the Tracker still work.                 │
│   Tailoring · Profile extraction · email reading  (disabled)  │
│   [ Connect an agent ]                                        │
└───────────────────────────────────────────────────────────────┘
```

AI-only actions render **disabled** (not hidden) with the reason and the **Connect an agent** call to
action; deterministic search, scoring and the Tracker keep working (FR-4).

### 9.15 Capability gating & locked store (P0-2 · P1-5)

**CLI unsupported for a task (P0-2).** Untrusted-content tasks (extract postings · classify email ·
tailor · review · narrate scores) run only on a CLI with **verified `tool_lockdown`**. Engine settings
shows a task × CLI matrix; unsupported cells are non-interactive, with **no override**.

```
Engine ▸ Agent CLI — task support                              [ Test all ]
┌────────────────────┬─────────────┬─────────────┬─────────────┐
│ Task               │ Claude Code │ Codex       │ Aider       │
├────────────────────┼─────────────┼─────────────┼─────────────┤
│ Extract postings   │ ✓ Supported │ ✓ Supported │ ✗ Unsupported│
│ Classify email     │ ✓ Supported │ ✓ Supported │ ✗ Unsupported│
│ Tailor / Review    │ ✓ Supported │ ✗ Unsupported│ ✗ Unsupported│
│ Narrate scores     │ ✓ Supported │ ✗ Unsupported│ ✗ Unsupported│
└────────────────────┴─────────────┴─────────────┴─────────────┘
Unsupported cells carry a plain reason; the fix is "Change CLI", never "run anyway".
```

```
Studio ▸ Tailor — blocked
┌─────────────────────────────────────────────────────────────────┐
│ ⛔ Tailoring needs an agent with tool lockdown.                  │
│    {CLI} can't run this task.                                    │
│    [ Change CLI ]                         [ Learn more ]         │
└─────────────────────────────────────────────────────────────────┘
```

**Locked store (P1-5).** With the passphrase path, the store is locked until unlocked **once per
login session** (zeroized memory + idle timeout). A scheduled run while locked is **skipped**, never
recorded as successful.

```
Topbar (locked)
┌─────────────────────────────────────────────────────────────────┐
│ Today                    🔒 Boba is locked — Unlock   [ Run now ]│
└─────────────────────────────────────────────────────────────────┘

Unlock dialog
┌───────────────────────────────────────────────────────┐
│ 🔒 Unlock Boba                                         │
│ Enter your passphrase to decrypt your data.           │
│ It stays in memory for this login session only.       │
│ [ ••••••••••••            ]            [ Unlock ]      │
└───────────────────────────────────────────────────────┘

Run report / tray
  Search Run · 08:00   ⏸ skipped — locked   "Boba is locked — enter your passphrase to run."
```

One OS notification fires on the first locked/skipped run (no repeat spam); unlocking offers
**Run now**. Topbar, tray, Settings ▸ Privacy and the Run report all reflect the locked state.

---

## 10. Responsive & Platform

- **Window floor:** 960×640. Below it, a calm "make the window a little bigger" floor — never a
  broken layout.
- **Breakpoint behaviors:** see `DESIGN.md` §10.4. Summary: sidebar expands ≥1280, rails 960–1279;
  inspector inline ≥1280 else drawer; kanban horizontal scroll (List mode <900); Offer grid 1→2→3 col.
- **macOS:** traffic-light inset in the window header; tray uses a template image (mono mark).
- **Windows / Linux:** window controls top-right; notification via native API; tray icon full color.
- **OS integrations:** tray, native notifications, launch-at-login (opt-in, default off), scheduler
  runs while logged in, missed-run catch-up (FR-43); all quiet, all pausable.
- **No mobile surface.** Responsive here means *window* responsiveness, not device classes.

---

## 11. Inspiration & Anti-patterns

**Inspiration (patterns, not pixels):** Linear's keyboard-first command palette; Things' calm
density; Apple's restraint on motion; a real ATS's posting realism (age, salary shown vs hidden).

**Anti-patterns to avoid**
- A single opaque "match %" — we ship two labeled scores, a Rank, and a Verdict with evidence.
- Color-only score communication — number + band word + shape glyph always accompany color.
- Silent VETOs — the quote and the override are always shown (FR-54).
- Gamification: streaks, confetti storms, application volume as pressure, "you're on fire".
- Hiding a poor Verdict behind green; hiding errors behind mascots.
- Fake urgency in notifications; per-Offer notification spam.
- Auto that feels uncontrolled — it is opt-in, rule-bound, dry-run-tested, Outbox-held, capped,
  logged, and Kill-Switch-stoppable.
- Forcing kanban on narrow windows; kanban without a List-mode fallback.
