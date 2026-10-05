# ADR-003 — Upstream reuse: re-implement the methodology, don't vendor the repo

- **Status:** Accepted · 2026-10-04
- **Spine:** AD-8, AD-14, AD-15, AD-19

## Context
`MadsLorentzen/ai-job-search` (MIT, snapshot `52f84e0`) is a Claude-Code-driven repo: Markdown commands and rubrics, Bun/TS portal CLIs, Python verification tools, LaTeX documents. Its value is the **methodology**: hard gates, the weighted rubric, the fresh-context reviewer, the fabrication guardrail, mechanical PDF/ATS verification, relevance-weighted cutting, and treating the posting as untrusted. Its runtime stack (Claude-only, Bun, Python, LaTeX) is the main barrier to installing it (`docs/research/upstream-analysis.md`).

## Decision
- **Re-implement**, don't vendor or submodule:
  - Gates and rubric go into `boba-domain::scoring`, extended to the two-axis model in offer-scoring.md.
  - The `/apply` flow goes into the agent-pack skills (`tailor_application`, `review_application`) plus core verification.
  - `verify_pdf`/`verify_layout` become Typst layout introspection plus pdf-extract.
  - `job_key`, `robots_check` and the `rank_state` pattern are ported to Rust.
- **Keep interoperable formats.** Exports are compatible with `job_search_tracker.csv`, `seen_jobs.json` and the per-application folders. The status vocabulary is the same. A future `external_process` adapter runs upstream's portal CLI contract (`search`/`detail --format json`).
- **Retire** the Claude-specific commands and hooks, the upstream-maintenance tools, mandatory LaTeX, and the Danish portals (market mismatch).
- Keep the **MIT notice** in `NOTICE` and `agent-pack/NOTICE`. Prompts are rewritten, not copied verbatim.

## Consequences
- We don't inherit upstream churn. We track it manually: an optional watch on the upstream CHANGELOG, reviewed by maintainers.
- Users of upstream can migrate through the import/export formats.
- Re-implementation costs effort, mostly in the scoring and verification ports, but removes three runtimes from the install.

## Alternatives
- **Vendor upstream and run it through Claude Code.** Claude-only, and still needs Python, Bun and LaTeX. Rejected.
- **Git submodule plus adapters.** Couples us to the upstream file layout and its Claude-specific prose. Rejected.
