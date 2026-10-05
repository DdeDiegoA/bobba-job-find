# ADR-006 — Document rendering: Typst (PDF) + docx-rs (DOCX) from one model

- **Status:** Accepted · 2026-10-04
- **Spine:** AD-14, AD-15

## Context
We need ATS-friendly PDFs, DOCX export, multiple templates, and bilingual ES/EN output. Upstream uses LaTeX (lualatex and xelatex, several GB to install) plus Python verification of the rendered PDF: exact page budgets, no orphaned headings, a readable text layer. Non-technical users cannot be asked to install TeX.

## Decision
- The agent produces a structured **`CvDocument` / `CoverLetterDocument`** (JSON Schema v1). Each claim-bearing item carries `fact_ids[]` (fabrication guardrail) and missing keywords go to `declared_gaps[]`.
- **PDF:** the **typst 0.15** crate is embedded in the core. Templates (`templates/{cv,cover}/*.typ`) read the JSON from `sys.inputs`. Fonts are bundled and loaded from memory (OFL: Lato, Source Sans 3, Noto Sans), so output is the same on every OS.
- **Verification:**
  - **Layout:** page count, orphaned headings and the signature block, read from Typst's laid-out document (this replaces `verify_layout.py`).
  - **ATS:** the text layer is extracted with pdf-extract (contact details literal, backed keywords present, no `(cid:`/U+FFFD). This replaces `verify_pdf.py`.
  - **Overflow:** a relevance-weighted cut, as a domain function (the upstream rule).
- **DOCX:** **docx-rs 0.4** writes from the same model with ATS-safe styles. It goes through the same fabrication check. Layout is not checked because Word reflows.
- **Language:** the content language follows the posting's language (lingua). Untranslated facts trigger a `translate_profile` proposal that the user approves first.

## Consequences
- There is no external toolchain, and rendering takes milliseconds, so the agent and verifier can iterate cheaply.
- Templates must be written in Typst. User-supplied LaTeX templates are deferred (they would require the user to install TeX).
- The PDF and DOCX can differ in layout but never in content.

## Alternatives
- **LaTeX** (upstream). Installation is too heavy. Deferred as an optional advanced template path.
- **HTML to PDF via headless Chrome.** Needs a browser present, pagination is less deterministic and layout can't be introspected.
- **docx first, then converting to PDF** (LibreOffice). Huge dependency.
- **Pandoc.** GPL external binary.
