# Boba Job Find 🧋

**A calm, local-first job search that runs itself every day, and keeps the decisions with you.**

Boba Job Find is a free, open-source desktop app for Windows, macOS and Linux. It looks
through the job boards you choose, ranks every Offer by how well it fits you *and* how good
it is, helps you tailor a CV and cover letter for each application, and applies for you in
the way you pick: you review each one, or Boba sends the ones that clear your rules.

It is a productized, cross-platform take on the manual workflow in
[MadsLorentzen/ai-job-search](https://github.com/MadsLorentzen/ai-job-search).

## What it does

- **Daily search.** A scheduled run fetches new Offers from your Sources, removes duplicates,
  and tells you what is worth a look. Nothing new? It says so, calmly.
- **Honest ranking.** Every Offer gets a **Fit Score** (can you get it?) and a **Quality Score**
  (do you want it?), combined into a **Rank** and a **Verdict** (Strong, Good, Moderate, Weak,
  Poor). You can see the breakdown and change the weights. Scam and ghost-posting badges
  warn you early.
- **Studio.** Tailored CV and cover letter per application, in English or Spanish to match the
  posting. Every line is traced back to a fact in your Profile, so Boba does not invent
  experience. Export ATS-friendly PDF or DOCX.
- **Apply your way.** Email Apply from your own mailbox, ATS form assist in your own Chrome,
  or an Apply kit for manual applications. **Review** mode keeps a human on every send;
  **Auto** sends only what clears your Auto Rules.
- **Tracker.** A kanban of your applications, follow-up nudges, a duplicate-application guard,
  and reply reading (opt-in) that proposes status changes you confirm.
- **Your tray, your schedule.** Runs in the background from the system tray, with native
  notifications and quiet hours.
- **English and Spanish** UI from day one, light and dark themes that follow your OS.

## Privacy: it stays on your machine

- No account, no cloud, no telemetry, no billing.
- Your data lives in one encrypted local database (SQLCipher). The key is held in your OS
  keychain. If no keychain is available, Boba asks for a passphrase instead.
- Update checks are **off** until you turn them on, as are the optional extras (on-device model
  download, salary exchange rates). Every network contact appears in
  **Settings → Privacy → Network activity**.
- Export your data in open formats, make a passphrase-encrypted backup, restore it, or
  **Reset everything** at any time.

Details: [Privacy, backup and reset](docs/user-guide/privacy-and-backup.md).

## Supported platforms

| OS | Installer |
|---|---|
| macOS 13+ (Apple Silicon and Intel) | Universal `.dmg` |
| Windows 10/11 x64 | NSIS `-setup.exe` or `.msi` |
| Linux x64 | `.AppImage`, `.deb`, `.rpm` |

Builds are not signed by Apple or Microsoft yet, so the first launch needs a one-time
approval. See [docs/install/](docs/install/README.md) and the
[troubleshooting guide](docs/user-guide/troubleshooting.md).

## The AI engine: bring your own agent CLI

Boba does not bundle a model and has no AI account of its own. It detects the agent CLIs you
already have installed (Claude Code, Codex, OpenCode, Gemini CLI, Aider) and lets you pick one
plus a model during setup. A specialized job-search agent (prompts and skills in
[`agent-pack/`](agent-pack/)) runs on that CLI.

The agent only *proposes*. It talks to the app through a narrow, sandboxed tool shim, and the
Rust core decides what actually happens: scoring, caps, sending, and everything else that
matters. Tasks that read untrusted text, such as job postings, only run on a CLI whose tool
lockdown Boba could verify for that run. If your CLI can't pass that check, Boba says so and
keeps working in **Limited mode** (search, scoring and the Tracker still work).

## Safety: Review, Auto, and the Kill Switch

- **Review mode** is the default: Boba prepares, you press Send.
- **Auto** is off until you add at least one Auto Rule and confirm. Each rule sets minimum
  Rank/Fit/Confidence, the allowed channels, active hours and caps. Defaults: 5 a day per rule,
  10 a day across all rules, 1 per company per 30 days; automatic sends are 5-20 minutes apart.
- A new rule starts as a **dry run** that only reports what it would have sent.
- Automatic sends wait in the **Outbox** first (hold between 5 minutes and 24 hours) so you can
  edit or cancel.
- The **Kill Switch** (in the app and in the tray) stops every automatic send immediately.
- Only documents that passed Boba's checks (facts, layout, ATS text layer) can be sent.
- Job boards without a public API (LinkedIn, Indeed) work only through *your own* signed-in
  browser session, per Source, behind an explicit opt-in with a terms-of-service and ban-risk
  warning, a separate opt-in for Auto, and much stricter limits.

Read more in [Applying](docs/user-guide/applying.md).

## Build from source

Prerequisites: Rust 1.99 (pinned by `rust-toolchain.toml`, installed automatically by
[rustup](https://rustup.rs)), Node.js 24 LTS, pnpm 11 (`corepack enable pnpm`), and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```sh
pnpm install          # JS dependencies
pnpm tauri dev        # run the app (Vite on http://localhost:1420)
pnpm tauri build      # production build and installers
```

Dev builds use a separate keychain service and data directory, so they never touch your real
data. The checks CI runs are listed in [CONTRIBUTING.md](CONTRIBUTING.md).

## Docs map

| You want to… | Read |
|---|---|
| Use the app | [docs/user-guide/](docs/user-guide/getting-started.md) |
| Install or update it | [docs/install/](docs/install/README.md) |
| Contribute | [CONTRIBUTING.md](CONTRIBUTING.md) |
| Report a security issue | [SECURITY.md](SECURITY.md), [docs/security/](docs/security/) |
| Understand the design | [docs/architecture/](docs/architecture/ARCHITECTURE-SPINE.md), [docs/design/](docs/design/DESIGN.md) |
| See what the product promises | [docs/product/PRD.md](docs/product/PRD.md) |
| Everything, indexed | [docs/README.md](docs/README.md) |
| See what changed | [CHANGELOG.md](CHANGELOG.md) |

## License and credit

[MIT](LICENSE). Boba Job Find is inspired by and builds on the ideas of
[MadsLorentzen/ai-job-search](https://github.com/MadsLorentzen/ai-job-search). Thank you to its
author for the workflow this app productizes.
