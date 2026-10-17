# ADR-015 — Migration numbers are assigned at rebase time

- **Status:** Accepted · 2026-10-16 (ruling during the build; first hit by S2.4 and S7.2)
- **Spine:** AD-2, AD-20

## Context
AD-20 stores forward-only migrations as `NNNN_name.sql`. The backlog (`slices.yaml`) planned a number per slice, but up to three slices build in parallel and merge in an order the plan cannot know. Two slices that planned `0011` would collide, and a reused or reordered number breaks the runner.

## Decision
- The numbers in `slices.yaml` are **advisory**.
- A slice takes the **next free number on `main` at rebase time**, just before merge. S2.4 took `0011`; S7.2 renumbered.
- Never edit a shipped migration (AD-20). A slice that renumbers does it only while its own migration is unmerged.
- The migration runner is the safety net: a gap or duplicate fails the store tests.

## Consequences
- Parallel slices do not need to coordinate numbers up front. The cost is one rename at rebase time.
- Code and tests must not hard-code a migration number from the plan.

## Alternatives
- **Reserve a number per slice in the plan:** wasted numbers and still wrong when a slice is dropped or split. Rejected.
- **Timestamp names:** loses the simple ordering the runner relies on. Rejected.
