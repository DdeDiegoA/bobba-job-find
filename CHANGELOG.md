# Changelog

All notable changes to Boba Job Find. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[semantic versioning](https://semver.org/).

## [1.0.0-beta.2] - 2026-10-18

Pre-release, like beta.1: the tag is `v1.0.0-beta.2`, so it never becomes
`/releases/latest`.

### Fixed

- AI engines when the app is launched from Finder: the probe now also searches
  the login-shell `PATH` and the known install dirs a GUI launch misses
  (`~/.opencode/bin`, and the pnpm global bin dir on Linux and Windows), so a
  CLI your shell finds is found here too (FIX1).
- The **Test connection** step reports actionable errors and offers Limited
  mode as the way on, instead of leaving the setup wizard stuck (FIX1).
- Settings navigation resolves every entry against `/settings`, so a second
  click no longer nests the URL and falls back to Preferences (FX1).
- The command palette runs the row you hover or arrow to, not the last row
  (FX2).
- The **Connections** page mounts Sources, Email and the session-board opt-in
  instead of the placeholder (FX3).
- The onboarding email step explains why the one-click buttons can be
  disabled, and offers Skip or bring-your-own-client under Connections > Email
  as the way on (FX5).
- The macOS DMG mounts without a licence agreement window, and the install
  steps match the quarantine path for the self-signed build:
  `xattr -dr com.apple.quarantine "/Applications/Boba Job Find.app"` (REL6).

### Release pipeline

- Releases run only after the CI gate passes (REL3).
- `latest.json` points at public release asset URLs so the updater can fetch
  them (REL5).
- Windows MSI gets a numeric WiX version derived from the tag
  (`v1.0.0-beta.2` → `1.0.0.2`) (REL2), and the release workflow uses `bash`
  on Windows (REL4).

## [1.0.0-beta.1] - 2026-10-18

First release (pre-release; the tag is `v1.0.0-beta.1`, so it never becomes
`/releases/latest`). Boba Job Find is a local-first, free and open-source
desktop app (Windows, macOS, Linux) that searches, ranks, tailors and applies,
with you in control. Grouped by epic.

### Foundations
- Tauri v2 shell with a hexagonal Rust core and a React 19 + TypeScript UI.
- Encrypted local database (SQLCipher); the key lives in the OS keychain, with a passphrase
  fallback and a **Locked** state when no keychain is available (unlock once per login, idle
  timeout, scheduled runs skip while locked and notify you).
- Typed IPC, error model and event bus; English and Spanish UI catalogues.
- App shell with light/dark theme following the OS, navigation, system tray, single instance
  and close-to-tray.
- Settings store and a hash-chained, tamper-evident audit log.
- CI pipeline: lint, types, tests, i18n parity, bindings freshness, clippy, `cargo deny`,
  multi-OS builds.

### Agent runtime
- Detects installed agent CLIs and probes their capabilities; you choose the CLI and model.
- Per-run sandbox and a `boba-mcp` stdio shim: the agent proposes, the core disposes.
- Hard **tool-lockdown gate**: tasks that read untrusted content run only on a CLI whose
  lockdown is verified for that run; otherwise Boba fails closed.
- Portable specialized agent pack (prompts and skills) and a task registry.
- Core-enforced run budgets and a token-usage display.

### Onboarding and Profile
- Setup wizard: language and theme, engine choice with a test call, master CV import with
  review, preferences, gap interview, sources, email, apply mode and first run.
- Profile of versioned facts with an editor.
- A forced, passphrase-encrypted **recovery backup** before onboarding finishes on builds that
  are not signed by Apple or Microsoft.
- **Limited mode** when no agent CLI is connected: search, scoring and the Tracker still work.

### Sources and ingestion
- Source SDK, registry and a hardened HTTP client (per-host rate limits, no redirect bypass,
  https-only apply links).
- ATS boards: Greenhouse, Lever, Ashby, Workable, SmartRecruiters.
- Remote feeds and aggregators, including Remotive, RemoteOK, We Work Remotely, Himalayas,
  Arbeitnow, Hacker News, Adzuna and Jooble.
- Custom RSS/Atom feeds, manual Offer intake (URL or pasted text).
- Offer normalisation and language detection, deduplication into clusters, repost, staleness
  and expiry handling.
- Connections screen with a catalogue, risk labels and source health.

### Scheduler and daily search
- In-app scheduler with a daily search run, run report and top-matches notification.
- Digest notifications with quiet hours; launch at login (opt-in) and tray run controls.

### Ranking and matches
- Gates, deterministic offline **Fit** and **Quality** scores, **Rank** and **Verdict**,
  versioned score cards with plain-language explanations.
- Today and Inbox, Offer detail with score breakdown, keyboard triage and a command palette.
- Adjustable scoring weights, and scam and ghost-posting badges.
- Optional on-device embeddings (opt-in download, checksum verified before load).

### Tailoring studio
- Tailored CV and cover letter per application, language matched to the posting (EN/ES).
- Fact-ID traceability and a fabrication validator; reviewer pass and revision.
- Layout verification with relevance-weighted cutting, ATS text-layer and keyword coverage
  checks, Typst PDF and DOCX export, two templates.
- Studio UI with editing, regenerate, diff and screening answers with an approved answer bank.

### Email
- Microsoft Graph (read and send) with loopback OAuth (PKCE), Gmail send-only, IMAP/SMTP.
- Scoped reply sync and manual `.eml` import; reply classification into Status Proposals you
  confirm; Gmail read via your own OAuth client (bring your own).

### Apply and Tracker
- Kanban Tracker, duplicate-application guard and follow-up nudges.
- Apply channels: Manual (with an Apply kit), Email Apply, ATS form assist in your own Chrome
  (never presses submit, never solves a CAPTCHA).
- Review mode and **Auto** with Auto Rules, apply policy engine, caps, dry run and **Outbox**
  hold.
- **Kill Switch** that stops every automatic send.
- Audit Log viewer and funnel stats.

### Data, privacy and release
- Open-format export, encrypted backup, validated restore, and reset that wipes the database,
  keychain entries and browser sessions.
- Retention sweep and a Network activity view that lists every contact the app makes.
- Release pipeline with minisign-verified updates (opt-in checks), self-signed macOS identity,
  and installers for all three OSes.

### Session boards (opt-in, higher risk)
- LinkedIn and Indeed adapters that use your own signed-in browser session, per Source, behind
  a terms-of-service and ban-risk warning, with sign-out and cookie clearing.
- Session Board apply in Review mode.
- Session Board **Auto** as a separate opt-in: 3 a day by default (10 at most), at least 10
  minutes apart, Outbox hold of at least 60 minutes, paused 7 days after any challenge, and
  re-confirmed every 30 days.

### Fixes after v1

- CI: install the Tauri Linux prerequisites in the Rust gate so the GitHub
  Actions runners are green (CH15), and give the web job the same deps while
  pinning `ubuntu-24.04` (CH16).
- Money: scale amounts by each currency's ISO-4217 minor-unit exponent, and pin
  fraction digits to it, so KWD/BHD/CLP and other zero/three-decimal currencies
  display correctly (CH15); salary display now goes through `formatMoney`
  (CH17).
- Windows: gate the Unix-only process group so the Windows build compiles, and
  prove the tree kill on Windows too (CH18).

### Known limitations
- macOS builds are **self-signed**, not notarised: Gatekeeper stops the first
  launch and needs a one-time approval
  ([docs/install/macos.md](docs/install/macos.md)).
- Windows builds are **unsigned** until SignPath is accepted: SmartScreen shows
  "Windows protected your PC" on first launch
  ([docs/install/windows.md](docs/install/windows.md)).
- The bundled Microsoft Graph and Google OAuth clients are **not registered
  yet**, so connecting email needs your own OAuth client ID (bring your own) or
  IMAP/SMTP ([docs/install/RELEASING.md](docs/install/RELEASING.md)).
- InfoJobs and further sources, and coaching extras, are planned for v1.1.

