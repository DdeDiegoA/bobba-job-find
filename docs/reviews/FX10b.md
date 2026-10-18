# FX10b review — Suggest improvement in the Profile, confirm one at a time

> **Reviewer:** reviewer-3 (qa) · **Dev:** dev-3 · **Date:** 2026-10-18
> **Branch:** `hermad/FX10b` · **Reviewed commit:** `a489a72` (`feat(profile): suggest fact improvements from the UI, confirm one at a time (FX10b)`)
> **Security-sensitive:** yes (agent output becomes fact text, confirm path, audit). Needs the human gate before merge (AGENTS.md).
> **Verdict:** CHANGES REQUESTED. Two defects reproduced (one keyboard/focus, one false copy). The security residuals R1, R4, R5 and R2/R3/R6 rows are closed as described below.

## Scope

- `src-tauri/crates/boba-app/src/profile/improve/mod.rs`: boundary and harvest (R1 evidence/detail ignored; R4 cap).
- `src-tauri/crates/boba-domain/src/profile/improve.rs`: `MAX_IMPROVEMENT_TEXT`, `text_within_cap`.
- `src/features/profile/improve.tsx` and `improve.test.tsx`: the suggestion panel.
- `src/features/profile/fact-form.tsx`: mounts the panel for an existing fact.
- `locales/{en,es}/profile.json`: `improve.*` keys.
- `docs/security/residuals.md` (R2, R3, R6), `agent-pack/skills/improve_fact/SKILL.md`.

## Residuals from FX10a

| R | Requirement | Result | Evidence |
|---|---|---|---|
| R1 | Evidence and detail are the fact's own, or shown before confirm | **Closed in core.** The run's `evidence` and `detail` are ignored at the boundary and in harvest. The confirmed fact keeps its own. | `a_suggestion_keeps_the_facts_own_evidence_and_detail` (passes); boundary test asserts `evidence` is ignored |
| R4 | Core length cap on agent text | **Closed.** 2 000 characters per variant, counted in chars, refused at the tool boundary (retryable) and again in harvest. A refused call does not fail a run whose retry succeeds. | `an_over_long_suggestion_is_refused_at_the_tool_boundary`, `a_refused_suggestion_can_be_retried_in_the_same_run`, domain `a_suggestion_longer_than_the_cap_is_refused` (all pass) |
| R5 | The UI is the only confirm path, per suggestion | **Closed.** `profileImproveConfirm` has one production caller, `improve.tsx`, and it passes the suggestion's own id. Rendering never confirms (test asserts it). | `grep profileImproveConfirm src` (only `improve.tsx`); `accepts exactly the suggestion that was rendered` |
| R2 | Stale check and save in one transaction | Recorded as `R-FX10a-2` in `residuals.md`. Accepted residual, unchanged. | `residuals.md` line 38 |
| R3 | Confirmed text is user text | Recorded as `R-FX10a-3`. | `residuals.md` line 52 |
| R6 | Lockdown cost of an improvement run | Recorded as `R-FX10a-6`. | `residuals.md` line 53 |

`FactForm` is keyed by `item_id` (`src/features/profile/index.tsx`), so suggestions cannot show on another fact.

## Acceptance criteria (dev's list: AC1–AC4)

| AC | Result |
|---|---|
| AC1 suggestion shown next to the current text, verbatim, as plain text | Pass (`renders agent text verbatim, never as markup`) |
| AC2 per-suggestion accept/reject; accept names only that suggestion | Pass |
| AC3 Limited mode disables the action with its own copy; errors translated | Pass (`disables the action without an engine and says why`, `shows a translated error`) |
| AC4 keyboard-operable, labelled, axe clean | **Partial.** Axe passes; keyboard focus is lost after accept/reject (defect 1). Button names repeat (defect 3). |

## Findings

### 1. High (a11y, keyboard): focus is lost after Accept or Reject

- **Snippet:** `improve.tsx` `pending.map(...)` renders the card as the list item. On success `drop(suggestion.id)` removes it from the DOM. Nothing moves focus, so focus falls to `document.body`.
- **Failure:** a keyboard user presses Accept (or Reject) on a card and their position in the page is lost. They must Tab from the top of the document again.
- **Repro:** render `ImproveFact`, run a suggestion, focus the Accept button, click it, wait for the card to leave. `document.activeElement` is `body`; `container.contains(activeElement)` is `false`. Reproduced with a temporary test (since removed).
- **Expected:** focus stays in the panel. After a drop, move focus to the next suggestion's heading or Accept button, or to the "Suggest improvement" button when the list is empty.
- **Fix (small):** on drop, focus the next card or fall back to the trigger. Add a test asserting `activeElement` is inside the panel after accept and after reject.

### 2. Medium (correctness, copy): false "didn't suggest" message after accepting the last suggestion

- **Snippet:** `improve.tsx` `{ran && !start.isPending && pending.length === 0 ? … t("improve.empty") …}`.
- **Failure:** `ran` is true after any run, and `pending` is empty after the user accepts or rejects the last suggestion. The panel then says "Your AI engine didn't suggest a change." That is false: the engine did suggest, and the user accepted it.
- **Repro:** one suggestion, accept it. The empty copy is shown. Reproduced with a temporary test (since removed). No existing test asserts the copy, so the suite stays green.
- **Expected:** the empty copy shows only when the run returned zero proposals.
- **Fix (small):** set an `emptyRun` flag from `outcome.proposals.length === 0` in the start `onSuccess`, and render the copy on that flag, not on `pending.length`. Add a test: accept the only suggestion, and assert the empty copy is absent.

### 3. Low to medium (a11y): repeated "Accept" and "Reject" names

- **Snippet:** `ImprovementCard` renders `<Button>{t("improve.accept")}</Button>` and `t("improve.reject")` in every card.
- **Failure:** with two or more suggestions, a screen-reader user hearing "Accept, button" from a button list cannot tell which suggestion it accepts. The card's visual context is not in the name.
- **Expected:** each button's accessible description names its suggestion, for example `aria-describedby` pointing at the suggestion text, or an `aria-label` that includes it.
- **Fix (small):** add `aria-describedby` on each button to the suggested text's `id`. Tests keep using `within(card)`, so they will not catch this without a new assertion.

## Gate (re-run by reviewer-3 on `a489a72`)

| Check | Result |
|---|---|
| `pnpm typecheck` | Pass |
| `pnpm lint` (Biome) | 0 errors. 3 warnings, all `noExplicitAny` in generated `src/lib/ipc/bindings.ts`, which this commit does not touch. Pre-existing generator output. |
| `pnpm test` | Pass: 90 files, 760 tests |
| `node scripts/check-i18n.mjs` | Pass (EN/ES aligned, no unused keys) |
| `node scripts/ci/check-conventions.mjs` | Pass |
| `node scripts/ci/check-workflow-policy.mjs` | Pass |
| `node scripts/gen-bindings.mjs --check` | Pass (up to date) |
| `cargo fmt --all --check` | Pass |
| `cargo clippy -p boba-app -p boba-domain --all-targets -- -D warnings` | Pass |
| `cargo test -p boba-app improve` | Pass: 18 tests |
| `cargo test -p boba-domain improve` | Pass: 16 tests |
| Full `cargo test --workspace`, `cargo deny check` | Not re-run by reviewer-3 here; dev-3 reports both green. Re-run before approval. |

## Verdict

**Changes requested.** Fix findings 1 and 2 (both reproduced, both need tests), and address 3. Then re-run the gate and re-review. The core work for R1, R4 and R5 is sound and can stay as it is.

Once the fixes land and this review passes, the commit goes to the orquestador with `APPROVED FX10b commit <sha>`.
