# FX8 review: draft the missing language with AI

> **Reviewer:** reviewer-3 (qa) · **Dev:** dev-3 · **Date:** 2026-10-18
> **Branch:** `hermad/FX8` · **Reviewed commit:** `3f5e3ec` (`feat(profile): draft the missing language with AI (FX8)`), on main `e32544f`
> **Verdict:** APPROVED at `ea196ac` (fix for F1, F2, F3 on top of `3f5e3ec`; initial verdict was CHANGES REQUESTED)

## Re-review at `ea196ac`

| Finding | Fix | Verified by |
|---|---|---|
| F1 (major) | `ProposalView.item_id` added (Rust, `profile_import/mod.rs`), bindings regenerated. `usePendingDrafts` reads `profile_import_proposals` and filters by `item_id`. New drafts are appended (`mergeDrafts`), not swapped in. Confirm and Discard drop the draft and invalidate the query. | 4 new profile tests (reopen shows drafts, confirmed draft not offered again, earlier draft kept on second request), 2 Rust tests on `item_id` mapping |
| F2 (minor) | Error is `role="alert"`; running state is `role="status"`. | "announces that a draft is running" |
| F3 (minor) | Notice shows whenever `!available && !isLoading`, including a failed snapshot. | "explains the disabled draft when the capability snapshot cannot be read" |

`item_id` is the fact's own ULID, which `FactView` already exposes to the UI. It carries no sensitive data.

Gate at `ea196ac`:

| Check | Result |
|---|---|
| `cargo fmt --all --check` | ok |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | ok |
| `cargo test -p boba-app --locked` | ok |
| `pnpm lint` / `pnpm typecheck` | ok (same 3 pre-existing binding warnings) |
| `pnpm vitest run` on profile, onboarding, proposal-review | 16 files, 144 tests passed (was 139) |
| `check-i18n`, `check-conventions`, `gen-bindings --check` | ok |

The full web suite was not re-run here; dev-3 reports 719/719 including e2e/a11y, which I have not verified.

## Gate (web half, in the worktree)

| Check | Result |
|---|---|
| `pnpm lint` | exit 0; 3 warnings, all in generated `src/lib/ipc/bindings.ts` (pre-existing, not FX8) |
| `pnpm typecheck` | ok |
| `pnpm vitest run` on profile, onboarding, proposal-review | 16 files, 139 tests passed |
| `node scripts/check-i18n.mjs` | ok: EN/ES aligned, ICU valid, no unused keys |
| `node scripts/ci/check-conventions.mjs` | ok |
| `node scripts/gen-bindings.mjs --check` | up to date (FX8 adds no command) |

No cargo run, per the orquestador's instruction. The FX8 commit touches no Rust.

## Acceptance criteria

| AC | Test | Result |
|---|---|---|
| AC1: badge for missing language + "Draft in <lang> with AI" button | `profile.test.tsx` "flags the missing language and offers a draft" | pass |
| AC2: draft via `profileGapsTranslate`, shown verbatim, editable, nothing saved | "drafts through the agent runtime and shows the proposal verbatim, unsaved" (asserts no `profileGapsConfirm`/`profileSaveFact`) | pass |
| AC3: Confirm saves a new version; Discard writes nothing | "saves the confirmed draft…", "discards a draft…" | pass for the single-draft path; see F1 |
| AC4: Limited mode disables the button, "no engine" copy, link to Settings > AI engine | "disables the draft, explains why…", "navigates from Limited mode…" | pass; see F3 |
| Run error: translated error, profile unchanged | "shows the translated error and leaves the profile unchanged…" (agent_unsupported) | pass |
| Gaps step reuses shared card; its tests pass unchanged | `gaps.test.tsx` 9 tests pass; `proposal-review.test.tsx` 6 tests | pass |

## Checks requested by the orquestador

- **No LLM output saved without explicit confirm.** Pass. The draft path ends in `proposal_append` only (`src-tauri/crates/boba-app/src/profile_gaps/service.rs` `harvest_translations`). The only write is `profileGapsConfirm`, which the user triggers. The card sends the proposal's text unchanged or the user's own edit. The harvest stores only the missing language, and `fact.draft()` keeps the other language as it already was, so Confirm cannot overwrite the existing variant.
- **Limited mode.** Pass for the enabled/disabled state, with F3 as a caveat.
- **i18n en + es.** Pass. `fact.missing.{en,es}`, `drafts.*`, `sections.*` and `gated:notice.limited*` all exist in both languages. Keys are built dynamically (`fact.missing.${missing}`), and check-i18n does not verify those, but I confirmed each one by hand.
- **a11y.** Mostly pass: the notice is `role="status"`, the textareas are labelled, and the card is plain text (`whitespace-pre-wrap`, no markup). See F2.
- **Untrusted text.** The card renders the proposal verbatim as text and shows the `untrusted_origin` warning. The translate input is built with `InputTrust::User`, which is correct for the user's own fact.

## Findings

### F1 (major): a draft can be stranded or orphaned, and the Profile still counts it

- **What happened:** `src/features/profile/ai-draft.tsx:38` keeps the drafts in local state (`useState<ProposalView[]>([])`), so they exist only while the fact form is open. The core stores each draft as a pending proposal (`harvest_translations`, `proposal_append`). The Profile then reports it: `src/features/profile/index.tsx:103-106` shows "N suggested facts are waiting for your review". No screen lists the pending drafts for a fact again. Also, `ai-draft.tsx:75` calls `setPending(outcome.proposals)` on each run, so a second draft replaces the first run's card and leaves its proposal pending and invisible.
- **Repro:** Open a one-language fact, click "Draft in Spanish with AI", then Cancel the form. Reopen the fact. The draft is gone, the button is back, and the Profile still says "1 suggested fact is waiting for your review". Also: draft twice in one open form; the first card disappears.
- **Expected:** Reopening a fact shows that fact's pending drafts, so they can be confirmed or discarded. A new draft is appended, not swapped in. The count matches what the user can act on.
- **Suggested fix (dev's call):** on open, load pending proposals with the existing `profile_import_proposals` command (registered in `command_list.rs`, returns `Vec<ProposalView>`) and filter by `item_id`. Append new drafts instead of replacing. A test for the reopen and the two-draft cases.
- **Note:** `service.rs:376-379` harvests proposals before it checks `failure(&events)`, so a run that writes a draft and then fails returns an error while the draft stays pending. This is core code outside the FX8 file list. The F1 fix covers it once the drafts are listed again, so I am not asking for a core change here.
- **Test gap:** no test covers reopen or a second draft.

### F2 (minor, a11y): the draft error is not announced

- `src/features/profile/ai-draft.tsx:98` renders `errorText` as a plain `<p>`. A screen reader user gets no announcement when a draft fails. Use `role="alert"`. The drafting state change (`drafts.drafting`) is also silent; a `role="status"` region would cover it.

### F3 (minor, Limited mode): no copy when the capability snapshot fails

- `ai-draft.tsx:80` shows the "no engine" notice only when `access.reason` is non-null. If the capability snapshot fails, `useCapability` returns `reason: null`, so the button is disabled with no explanation, which is the silent-failure case the plan forbids. Show the limited copy whenever `!access.available` and the snapshot has loaded (`!access.isLoading`).

## Summary for dev-3

1. Fix F1: list the fact's pending drafts on open, append new drafts, and add tests for reopen and two drafts.
2. Fix F2 and F3, with tests.
3. Re-run the web-half gate and the three checks above. Commit and tell reviewer-3.
