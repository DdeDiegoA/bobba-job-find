# Getting started

Welcome. Setup takes a few minutes, your answers are saved as you go, and you can change
every one of them later.

## 1. Install

Pick your OS in [docs/install/](../install/README.md). Because the installers are not signed
by Apple or Microsoft yet, your computer will ask you to approve the first launch once. The
[troubleshooting page](troubleshooting.md) walks through it.

You also need an **agent CLI** on your machine (Claude Code, Codex, OpenCode, Gemini CLI or
Aider). Boba does not bundle a model. No CLI yet? You can still finish setup and use Limited
mode (see below).

## 2. The setup wizard

On first launch Boba opens the wizard. If you leave halfway, it offers to **resume where you
left off**.

1. **Language and theme.** English or Spanish; light, dark, or follow your system.
2. **Engine.** Boba lists the agent CLIs it found. Pick one and a model, then run the **test
   call** to see that it works. Boba checks, for each run, that the CLI's tool lockdown is
   verified before it reads untrusted text such as job postings.
3. **Master CV.** Import your CV (PDF, DOCX or text) and review what Boba understood. These
   become the facts in your **Profile**, the only source for everything Boba writes about you.
4. **Preferences.** Roles, seniority, salary floor (hard or soft), locations, languages and the
   things you want to avoid. Ranking follows these.
5. **Gap interview.** A short Q&A for things your CV doesn't say yet, drafted in English or
   Spanish.
6. **Sources.** Safe Sources are already on for your markets. Add your own RSS or Atom feed if
   you like. Session boards (LinkedIn, Indeed) stay off until you opt in; see
   [Sources and connections](sources-and-connections.md).
7. **Email (optional).** Connect the mailbox Boba sends from, or skip and do it later.
8. **Recovery backup.** See below.
9. **Apply mode and first run.** Start in **Review** mode. Boba runs your first search so you
   land on a Today screen with real matches.

You can skip most steps and finish them later from **Profile**, **Connections** and
**Settings**.

## The recovery backup

Boba's data is encrypted and stays on your computer. That is great for privacy, and it means
that if this machine is lost, nobody, including us, can recover it for you. So the wizard asks
you to save a **recovery backup**: one file, encrypted with a passphrase you choose, that opens
on any system.

- On builds that are not signed by Apple or Microsoft, onboarding cannot finish and no search
  runs until a recovery backup exists.
- Choose where to save it (somewhere other than this computer is best) and a passphrase. Boba
  **cannot recover that passphrase**, so store it in your password manager.

More in [Privacy, backup and reset](privacy-and-backup.md).

## Limited mode

If no agent CLI is connected, or your CLI can't pass the safety check for untrusted content,
Boba tells you and switches to **Limited mode**: search, scoring and the Tracker keep working.
Tailoring, Profile extraction and reply reading wait until you connect a CLI that can do them
safely (**Settings → AI engine**).

## Your daily rhythm

After setup, Boba runs a search on schedule and sends a native notification when something is
worth a look. Open **Today** to see your closest matches and **Inbox** to triage. Boba keeps
running from the tray when you close the window; quit from the tray menu when you're done.

Next: [Sources and connections](sources-and-connections.md).
