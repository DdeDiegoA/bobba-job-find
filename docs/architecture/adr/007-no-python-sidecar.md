# ADR-007 — No Python sidecar; `boba-mcp` is the only sidecar

- **Status:** Accepted · 2026-10-04
- **Spine:** AD-1

## Context
AGENTS.md left a Python sidecar open "if upstream reuse warrants". Upstream's Python is about 3.6k lines of mostly standard-library tools: PDF verification, layout measurement, state, dedup keys, robots.txt, salary lookup, and Excel conversion. The scoring model also needs on-device embeddings (offer-scoring.md, F3).

## Decision
**No Python at runtime.** Each tool is ported to Rust or has a direct crate equivalent:

| Upstream tool | Rust equivalent |
|---|---|
| `verify_pdf` | pdf-extract |
| `verify_layout` | Typst introspection |
| `job_key`, `rank_state` | domain code |
| `robots_check` | texting_robots |
| `salary_lookup` | strsim |
| `convert_salary_excel` | calamine |
| embeddings | fastembed / ONNX Runtime with multilingual-e5-small |

The only sidecar is **`boba-mcp`**, a small Rust stdio MCP server shipped via `bundle.externalBin`, so it is signed with the app.

## Consequences
- One toolchain to build, sign and notarize. Smaller installers and faster startup.
- Python remains welcome for **dev-time** scripts (data refreshes, fixtures) under `scripts/`, run with `uv`.
- **Revisit trigger:** a required capability with no Rust or ONNX path. In that case, use python-build-standalone + uv as an `externalBin`, speaking stdio JSON-RPC, never as a network server.

## Alternatives
- **PyInstaller sidecar.** 30–80 MB per OS, antivirus false positives on Windows, and a second notarization surface. Rejected.
- **Require system Python.** Fails the non-technical install goal. Rejected.
