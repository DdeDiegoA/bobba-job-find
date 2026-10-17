# Architecture Decision Records

Format: Context → Decision → Consequences → Alternatives considered. Status: `Accepted` unless marked otherwise. Items marked **⚠️ Approval** touch auth, money, the DB or security, and need Diego's sign-off (via the orquestador).

| ADR | Title | Status | Spine ADs |
|---|---|---|---|
| [001](./001-desktop-shell-tauri-v2.md) | Desktop shell: Tauri v2 | Accepted | AD-1, AD-22 |
| [002](./002-engine-via-agent-cli.md) | AI engine via user-installed agent CLIs + boba MCP | Accepted · Approved (Diego), amended by 010 | AD-4, AD-5, AD-6, AD-7 |
| [003](./003-upstream-reuse-strategy.md) | Upstream reuse: re-implement methodology, don't vendor | Accepted | AD-8, AD-14, AD-15, AD-19 |
| [004](./004-storage-encryption-sqlcipher.md) | Storage: SQLCipher + OS keychain | Accepted · Approved (Diego), amended by 009 | AD-2, AD-13, AD-20 |
| [005](./005-source-access-and-scraping-policy.md) | Source access classes, scraping and auto-apply policy | Accepted · Approved (Diego, session-board auto in MVP) | AD-9, AD-10, AD-11, AD-12 |
| [006](./006-document-rendering-typst-docx.md) | Document rendering: Typst (PDF) + docx-rs (DOCX) | Accepted | AD-14 |
| [007](./007-no-python-sidecar.md) | No Python sidecar; boba-mcp is the only sidecar | Accepted | AD-1 |
| [008](./008-ui-stack.md) | UI stack: React + Tailwind v4 + shadcn + TanStack Query + tauri-specta | Accepted | AD-3, AD-21 |
| [009](./009-key-custody-signing-and-locked-state.md) | DB key custody across unsigned builds; Locked state; snapshot & restore | Accepted · Approved (Diego) | AD-13, AD-18, AD-20, AD-22 |
| [010](./010-untrusted-content-requires-tool-lockdown.md) | Untrusted-content tasks require verified tool lockdown; agent output untrusted | Accepted · Approved (Diego) | AD-4, AD-6, AD-7 |
| [011](./011-audit-chain-canonical-form.md) | Audit chain: frozen canonical form and anchor semantics | Accepted | AD-12 |
| [012](./012-composition-root-and-port-wiring.md) | Composition root and port wiring | Accepted | AD-3, AD-14 |
| [013](./013-passphrase-kdf-record-and-session-key.md) | Passphrase KDF record (Argon2id) and the login-session key | Accepted · Approved (Diego standing OK) | AD-13, AD-18, AD-20 |
| [014](./014-per-run-lockdown-gate.md) | Per-run untrusted-content gate: pinned config plus the run's own init event | Accepted · Approved (Diego) | AD-4, AD-6, AD-7 |
| [015](./015-migration-numbering.md) | Migration numbers are assigned at rebase time | Accepted | AD-2, AD-20 |
| [016](./016-per-area-error-locale-files.md) | Core error keys live in per-area locale files | Accepted | AD-21 |
| [017](./017-event-names-without-dots.md) | Event names: `<area>:<verb-ed>`, no dots | Accepted | AD-3 |
