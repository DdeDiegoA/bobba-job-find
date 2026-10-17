# Boba Job Find — Tech Stack

> **Status:** v1, 2026-10-04 · **Owner:** architect · Binding decisions are in [`ARCHITECTURE-SPINE.md`](./ARCHITECTURE-SPINE.md). Rationale is in [`architecture.md`](./architecture.md) and [`adr/`](./adr/).
> **Versions** were checked against crates.io, the npm registry and the GitHub API on **2026-10-04**. Pin minor versions in manifests (`^x.y` for npm, `x.y` for Cargo) and commit the lockfiles. Licences are taken from the registry metadata.
> **Licence policy:** the app is distributed under MIT (Apache-2.0 compatible). Allowed (as in `deny.toml`): MIT, Apache-2.0 (also WITH LLVM-exception), BSD-2/3-Clause, ISC, Zlib, MPL-2.0 (file-level), OFL-1.1 (fonts), CC-BY-4.0 (data), CC0-1.0, Unicode-3.0, Unicode-DFS-2016, 0BSD. `CDLA-Permissive-2.0` (Mozilla CA root data, not code) is allowed only for `webpki-root-certs` and `webpki-roots`. Copyleft (GPL/AGPL/LGPL-static) is **not allowed**. `cargo deny` and a license-checker script enforce this in CI.

---

## 1. Shell and platform

| Tech | Version | License | Why | Rejected |
|---|---|---|---|---|
| **Tauri** (`tauri`, `@tauri-apps/cli`, `@tauri-apps/api`) | 2.12.x | MIT/Apache-2.0 | Locked by the user. Small binaries, Rust core, tray/notification/updater plugins, system webview. See ADR-001. | Electron (≈150 MB, Node main process, a second language for the core); Wails (Go, not in the team's skill set); Flutter/Qt (no React reuse) |
| **Rust** | toolchain 1.99 (`rust-toolchain.toml`), MSRV 1.92 (from typst) | MIT/Apache-2.0 | One language for all logic | — |
| **Node.js** (build only) | 24 LTS | MIT | Vite and Tauri CLI at build time. **Not shipped.** | Bun as the build tool (fine locally, but Node is the one tauri-action and the docs assume) |

### Tauri plugins (all 2.x, MIT/Apache-2.0)

| Plugin | Version | Use |
|---|---|---|
| tauri-plugin-single-instance | 2.5 | One scheduler, one DB writer (AD-18) |
| tauri-plugin-notification | 2.5 | Daily matches, apply results |
| tauri-plugin-autostart | 2.7 | Opt-in launch at login (tray background) |
| tauri-plugin-updater + tauri-plugin-process | 2.13 / 2.4 | Signed auto-update + relaunch |
| tauri-plugin-dialog | 2.8 | Native confirm, file pickers (export, CV import) |
| tauri-plugin-opener | 2.7 | Open apply links in the default browser |
| tauri-plugin-window-state | 2.5 | Remember window geometry |
| tauri-plugin-log | 2.10 | Local rolling logs (redacted) |
| tauri-plugin-global-shortcut | 2.4 | Global Kill Switch shortcut (architecture §10.2; story 1.1 AC8) |
| ~~tauri-plugin-deep-link~~ | — | **Not used.** OAuth is loopback-only (scheme squatting risk; planning audit P1-11). |
| ~~tauri-plugin-shell~~ | — | **Not used.** The core spawns processes itself (ProcessSupervisor). No shell access from the webview. |
| ~~tauri-plugin-sql / store / fs~~ | — | **Not used.** All persistence goes through the core (AD-2). |

---

## 2. UI (React)

| Concern | Choice | Version | License | Why | Rejected |
|---|---|---|---|---|---|
| Framework | **React** + react-dom | 19.3 | MIT | User strength. Locked. | — |
| Language | **TypeScript** | 7.0 | Apache-2.0 | Native compiler (fast `tsc`). We use Biome, not typescript-eslint, so we don't depend on the TS 7 API. | TS 6.x (fallback if a tool breaks) |
| Build | **Vite** + @vitejs/plugin-react | 8.3 / 6.1 | MIT | The Tauri template default. Fast HMR. | Next.js (SSR is pointless in a desktop app), Rsbuild |
| Styling | **Tailwind CSS** | 4.3 | MIT | UX DESIGN.md tokens map to `@theme` + CSS variables | CSS-in-JS (runtime cost, CSP friction), vanilla-extract |
| Components | **shadcn/ui** (CLI) on **Radix** (`radix-ui`) | 4.21 / 1.6 | MIT | Components are owned code, so the boba look is easy to apply. Accessible primitives. Agreed with UX. | MUI/Chakra/Mantine (heavy theming fights the playful look), Headless UI (fewer primitives) |
| Variants | class-variance-authority, tailwind-merge, clsx | 0.7 / 3.7 / 2.1 | Apache-2.0/MIT/MIT | shadcn conventions | — |
| Icons | **lucide-react** | 1.52 | ISC | UX choice | — |
| Fonts | **@fontsource/fredoka, nunito, jetbrains-mono** | 5.3 | OFL-1.1 | Self-hosted. No CDN (local-first, CSP `self`). | Google Fonts CDN (network call, privacy) |
| Server state | **TanStack Query** | 5.104 | MIT | Caches command results. Core events invalidate keys (AD-3). | Redux Toolkit Query (heavier), SWR (weaker mutation story) |
| UI state | **Zustand** | 5.0 | MIT | Small, for UI-only state (wizard step, panels). Never mirrors DB data. | Redux (boilerplate), Jotai (fine, but less familiar) |
| Routing | **React Router** (declarative mode) | 8.4 | MIT | The most familiar option. Few routes. Memory/hash history in Tauri. | TanStack Router (great types, but more to learn for little gain at ~8 routes) |
| Forms | **react-hook-form** + **zod** (+ @hookform/resolvers 5.9) | 7.89 / 4.6 | MIT | Wizard and settings forms. zod schemas mirror the Rust validation. | Formik (unmaintained pace), TanStack Form (younger) |
| i18n | **i18next** + **react-i18next** + **i18next-icu** | 26.4 / 17.0 / 2.4 | MIT | ICU plurals/gender for EN/ES. Catalogs shared with the core (AD-21). | FormatJS/react-intl (fine, but i18next has better namespace/lazy tooling); Lingui (macro build step) |
| i18n tooling | **i18next-cli** | 1.74 | MIT | Extract keys and check for missing ones in CI | i18next-parser (superseded) |
| Typed IPC | **tauri-specta** + **specta** | =2.0.0-rc.25 | MIT | Generates TS for commands *and* events. The de facto Tauri v2 standard, despite the long rc. Pin it exactly. | ts-rs 12 (stable, but only types, no command wrappers; this is the fallback); hand-written `invoke` (drift) |
| Charts | **Recharts** | 3.10 | MIT | Score-factor bars, pipeline stats. Simple and declarative. | visx (lower level), Chart.js (canvas, harder to theme with tokens), ECharts (heavy) |
| Kanban drag-and-drop | **@dnd-kit/core** (+ sortable) | 6.3 | MIT | Accessible (keyboard DnD), stable | @dnd-kit/react 0.5 (pre-1.0), react-beautiful-dnd (deprecated) |
| Tables / long lists | @tanstack/react-table, @tanstack/react-virtual | 9.2 / 3.14 | MIT | Matches list, audit log | AG Grid (overkill) |
| Toasts / command palette | sonner, cmdk | 2.0 / 1.1 | MIT | shadcn defaults | — |
| Dates | date-fns (+ `Intl`) | 4.4 | MIT | Tree-shakeable. `Intl` handles locale formatting. | Moment (legacy), Day.js (plugins sprawl) |

---

## 3. Rust core crates

| Concern | Crate | Version | License | Why | Rejected |
|---|---|---|---|---|---|
| Async runtime | **tokio** | 1.53 | MIT | Tauri's runtime | async-std (unmaintained) |
| Errors | **thiserror** (libs), **anyhow** (bins/tests) | 2.0 / 1.0 | MIT/Apache | Convention in the spine | — |
| Serialization | **serde**, **serde_json** | 1.0 | MIT/Apache | — | — |
| JSON Schema | **schemars** + **jsonschema** | 1.2 / 0.58 | MIT | MCP tool schemas; validating agent proposals (AD-5) | hand-written validation |
| IDs / time | **ulid**, **time** | 3.0 / 0.3 | MIT | AD-17 | uuid v4 (unsortable), chrono (heavier; time is enough) |
| Logging | **tracing**, tracing-subscriber | 0.1 / 0.3 | MIT | Structured, with redaction layer | log/env_logger |
| DB | **rusqlite** `bundled-sqlcipher-vendored-openssl` | 0.40 | MIT (SQLCipher BSD-3, OpenSSL Apache-2.0) | Encrypted SQLite, statically built on all OSes. See ADR-004. | sqlx (SQLCipher support awkward, compile-time checks need a DB); SeaORM/Diesel (ORM weight); libsql (encryption tied to Turso builds) |
| Migrations | **rusqlite_migration** | 2.6 | Apache-2.0 | Embedded SQL, uses `user_version` | refinery (fine, more machinery) |
| Read pool | **r2d2** + **r2d2_sqlite** | 0.8 / 0.35 | MIT/Apache | Read connections. The writer is a dedicated actor thread. | tokio-rusqlite only (single connection) |
| Keychain | **keyring-core** + `apple-native-keyring-store`, `windows-native-keyring-store`, `dbus-secret-service-keyring-store` | 1.0 / 1.0 / 1.1 / 1.0 | MIT/Apache | keyring v4 split, the recommended setup. macOS Keychain, Windows Credential Manager, Linux Secret Service. | tauri-plugin-stronghold (Tauri plans to remove it in v3; own vault file); plaintext config |
| Secret hygiene | **secrecy**, **zeroize** | 0.10 / 1.9 | MIT/Apache | AD-13 | — |
| KDF (fallback) | **argon2** | 0.6 | MIT/Apache | Passphrase key when there is no keychain | PBKDF2 (weaker) |
| Hashing | **blake3** | 1.8 | CC0/Apache | Audit chain, content hashes | sha2 (slower, fine) |
| HTTP | **reqwest** (rustls) | 0.13 | MIT/Apache | Standard | ureq (blocking), hyper (too low-level) |
| Rate limiting | **governor** | 0.10 | MIT | Per-host token buckets (AD-10) | hand-rolled |
| robots.txt | **texting_robots** | 0.2 | MIT/Apache | Small, correct parser (port of upstream `robots_check.py`) | robotstxt 0.3 (Google port, stale since 2021) |
| Feeds | **feed-rs** | 3.0 | MIT | RSS / Atom / JSON Feed | rss + atom_syndication (two crates) |
| HTML | **scraper** (parse), **ammonia** (sanitize), **html2text** | 0.27 / 4.2 / 0.17 | ISC/MIT·Apache/MIT | Posting HTML → safe HTML + text (AD-7) | — |
| JSONPath (custom JSON sources) | **jsonpath-rust** | 1.0 | MIT | User field mappings | serde_json_path (fine, less active) |
| Language detection | **lingua** | 1.8 | Apache-2.0 | Accurate on short text, ES/EN subset only (keeps the binary small) | whatlang (less accurate on short snippets) |
| Near-dup | **simhash** | 0.3 | MIT | Cross-source dedup (AD-19) | MinHash crates (overkill) |
| Embeddings (optional) | **fastembed** (ONNX via `ort`) — model `MultilingualE5Small` | 7.1 | Apache-2.0 | On-device F3 semantic score, CPU-only | Python sentence-transformers sidecar (ADR-007); remote embedding API (breaks local-first) |
| Scheduling | **cron** (expression parsing) + tokio interval loop | 0.17 | MIT/Apache | Simple, and catches up after sleep (AD-18) | tokio-cron-scheduler (stale, own job store) |
| Process supervision | **process-wrap** (tokio) | 10.0 | MIT/Apache | Process groups / Job Objects, kill-tree | command-group (superseded by process-wrap) |
| Local IPC | **interprocess** (local sockets / named pipes) | 2.4 | 0BSD/Apache-2.0, plus transitive 0BSD (doctest-file, recvmsg) | core ↔ boba-mcp | localhost TCP (any local process can connect; firewall prompts) |
| Binary discovery | **which**, **dirs** | 8.0 / 7.0 | MIT | Agent CLI and browser detection | — |
| OS locale | **sys-locale** | 0.3 | MIT/Apache | Default UI language | — |
| MCP | **rmcp** (official Rust MCP SDK) | 3.5 | MIT/Apache | Server side in boba-mcp, stdio transport | TS MCP SDK (would need Node at runtime) |
| Typed IPC | **tauri-specta** / **specta** | =2.0.0-rc.25 | MIT | See UI table | — |
| TS exporter | **specta-typescript** | =0.0.12 | MIT | The TypeScript backend tauri-specta rc.25 uses to write `src/lib/ipc/bindings.ts` (AD-3). Pinned exactly alongside tauri-specta, and upgraded only together with it. | — |
| PDF render | **typst** + **typst-pdf** (+ **typst-as-lib** for the World impl) | 0.15 / 0.15 / 0.16 | Apache-2.0 / MIT | Embedded, no external TeX, fast, deterministic, introspectable layout. See ADR-006. | LaTeX (GB install, upstream pain); headless Chrome print (needs a browser); wkhtmltopdf (dead); printpdf (too low-level) |
| PDF text (ATS check) | **pdf-extract** (+ **lopdf**) | 0.12 / 0.45 | MIT | Text-layer extraction to verify ATS readability | pypdf / Poppler (Python or native deps) |
| DOCX | **docx-rs** | 0.4.22 | MIT | Pure Rust writer | `docx` npm (would run in the UI and duplicate logic); pandoc (external binary, GPL) |
| CV import (DOCX/PDF read) | docx-rs reader + pdf-extract | — | MIT | Text extraction for `profile_import` | — |
| Browser automation | **chromiumoxide** (CDP) | 0.9 | MIT/Apache | Drives the user's own Chrome/Edge, no bundled browser, Rust-native. See ADR-005. | Playwright (needs Node runtime + ~150 MB browsers per OS); fantoccini (needs a WebDriver binary); Tauri webview automation (differs per OS, WebKitGTK blocked by boards, mixes untrusted origins into the app) |
| Email send | **lettre** (SMTP, XOAUTH2) | 0.11 | MIT | Standard | — |
| Email read | **async-imap** + **mail-parser** | 0.12 / 0.11 | MIT/Apache | IMAP + robust MIME parsing | — |
| OAuth | **oauth2** (PKCE, loopback) | 5.0 | MIT/Apache | RFC 8252 desktop flow | openidconnect (not needed) |
| Graph / Gmail | plain reqwest + serde models | — | — | Avoid heavy generated SDKs | graph-rs-sdk, google-gmail1 (huge generated crates) |

---

## 4. Python sidecar — **No** (ADR-007)

Every upstream Python tool has a small, direct Rust equivalent:

| Upstream tool | Rust equivalent |
|---|---|
| `verify_pdf.py` | pdf-extract |
| `verify_layout.py` | typst layout introspection |
| `job_key.py` | ~50 lines in `boba-domain` |
| `robots_check.py` | texting_robots |
| `salary_lookup.py` | strsim fuzzy match in `boba-domain` |
| `convert_salary_excel.py` | `calamine` 0.36 (MIT), if needed |
| embeddings | fastembed |

Bundling CPython (PyInstaller/python-build-standalone) would add 30–80 MB per OS, a second signing and notarization surface, and slower startup. **Revisit only if** a needed capability has no Rust or ONNX path (for example, a specific ML model). In that case, ship it as a Tauri `externalBin` built with python-build-standalone + `uv`, spoken to over stdio JSON. Never as a server.

The only sidecar is **`boba-mcp`** (Rust, about 2 MB).

---

## 5. Agent CLIs supported (runtime, user-installed — not bundled)

| CLI | Verified version | Tier | Notes |
|---|---|---|---|
| Claude Code | 2.1.289 (local) | Full | `-p --output-format stream-json`, `--strict-mcp-config`, `--allowedTools`, `--max-turns`, `--max-budget-usd` |
| Codex CLI | docs 2026-10 | Full/Limited | `codex exec --json --ephemeral`, `-m`, `--sandbox read-only`, `--output-schema`; MCP via `-c mcp_servers…` |
| Gemini CLI | docs 2026-10 | Full | `-p --output-format stream-json`, `-m`; MCP and excludeTools via workspace settings |
| OpenCode | 1.18.34 (local) | Full | `run --format json -m provider/model`; `opencode models`; permissions in workspace `opencode.json` |
| Aider | probe at runtime | Text-only (unless MCP probe passes) | `--message --yes-always --no-git` |
| Custom | — | Degraded | argv template |

Capabilities are **probed**, not assumed (AD-4). Each driver has a recorded-JSONL fixture test in CI.

---

## 6. Testing, lint, CI

| Concern | Tool | Version | License |
|---|---|---|---|
| TS lint + format | **Biome** | 2.5 | MIT/Apache |
| TS unit/component | **Vitest** + @testing-library/react | 5.0 / 16.3 | MIT |
| E2E (app) | **WebdriverIO** + **tauri-driver** (Linux/Windows; macOS has no WKWebView driver) | 9.32 / 2.1 | MIT / MIT·Apache |
| Rust lint | rustfmt, clippy (`-D warnings`) | toolchain | — |
| Rust tests | `cargo test` + `insta` 1.49 snapshots for golden ScoreCards | — | Apache-2.0 |
| HTTP fixtures | recorded JSON in `tests/fixtures/`, `wiremock` for adapter tests | 0.6 | MIT/Apache |
| Supply chain | **cargo-deny** (advisories, licences, bans, sources), `npm ci --ignore-scripts`, Dependabot | 0.20 | MIT/Apache |
| CI | GitHub Actions: `actions/checkout@v5`, `actions/setup-node@v5`, `dtolnay/rust-toolchain`, `Swatinem/rust-cache`, **`tauri-apps/tauri-action@v1`** (release) | — | — |

Rejected: ESLint + Prettier (two tools, slower, typescript-eslint lags TS 7), Jest (slower, ESM friction), Playwright for app E2E (cannot drive the Tauri webview on all OSes; tauri-driver is the supported path), Cypress.

---

## 7. Data and assets bundled

| Asset | License | Notes |
|---|---|---|
| ESCO tech-skills subset + synonyms | CC-BY 4.0 (EU) | Attribution in About and NOTICE |
| BLS OES extract (US) | Public domain | Yearly refresh script (dev-time) |
| Typst CV/cover templates | Ours (MIT) | Inspired by upstream layouts |
| CV fonts (e.g., Lato, Source Sans 3, Noto Sans) | OFL-1.1 | Embedded in templates. Noto covers ES diacritics. |
| UI fonts (Fredoka, Nunito, JetBrains Mono) | OFL-1.1 | Via @fontsource |
| multilingual-e5-small ONNX (optional download) | MIT (intfloat) | Checksum verified, opt-in (open Q4) |
| Upstream methodology (re-implemented) | MIT © Mads Lorentzen | NOTICE retained |
