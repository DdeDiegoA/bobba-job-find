# Changelog

All notable changes to Boba Job Find. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[semantic versioning](https://semver.org/).

## [1.0.0] - 2026-10

First release. Boba Job Find is a local-first, free and open-source desktop app (Windows,
macOS, Linux) that searches, ranks, tailors and applies, with you in control. Grouped by epic.

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

### Known limitations
- Installers are not signed by Apple or Microsoft; the first launch needs a one-time approval
  (see [docs/install/](docs/install/README.md)).
- InfoJobs and further sources, and coaching extras, are planned for v1.1.

[1.0.0]: https://github.com/diegoarenas/boba-job-find/releases/tag/v1.0.0
