# ADR-016 — Core error keys live in per-area locale files

- **Status:** Accepted · 2026-10-16 (CH7)
- **Spine:** AD-21

## Context
Every slice that returns an `AppError` adds `errors.<area>.*` keys to the EN and ES catalogs. With all of them in `common.json`, parallel slices conflicted on the same file on almost every merge.

## Decision
- Core-emitted error keys stay namespace-less (`errors.<area>.<key>`) but live one file per area: `locales/{en,es}/errors/<area>.json`, with the same key names in both languages.
- A slice creates or extends **its own** area file without asking. A **new** area also needs one line in `ERROR_FRAGMENTS` in `boba-app/src/i18n/mod.rs`.
- `errors.*` never goes into `common.json`. `scripts/check-i18n.mjs` rejects an error key left in a plain catalog file and a key missing in either language. The runtime loaders (i18next and the Rust `include_str!` embed) merge the fragments into the default namespace.

## Consequences
- Two slices touch the same error file only when they share an area.
- Adding an area has one extra registration line, caught by CI if forgotten.

## Alternatives
- **One merged `common.json`:** the conflict source this ADR removes. Rejected.
- **`merge=union` on JSON:** produces invalid JSON on a bad merge. Rejected.
