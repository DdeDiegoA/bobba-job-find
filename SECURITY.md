# Security policy

Boba Job Find handles your CV, your email access and, optionally, your job-board sessions, so
we take security reports seriously.

## Reporting a vulnerability

**Please report privately. Do not open a public issue for a vulnerability.**

Use GitHub's private reporting: open the repository's **Security** tab and choose
**Report a vulnerability** (a private security advisory). Include what you found, the steps to
reproduce it, the version and OS, and the impact you think it has. If you can't use GitHub
advisories, open a minimal public issue that says only that you have a security report and ask
for a private channel. Don't put details in it.

You can expect an acknowledgement, a fix plan, and credit in the release notes if you want it.
This is a volunteer-run open-source project, so please allow some time for a response.

## What counts

Examples we care about most: reading or tampering with the encrypted database or its key,
bypassing the tool lockdown or the sandbox around the agent CLI, prompt-injection that makes
Boba send or leak something, bypassing the Kill Switch, caps or the Outbox, OAuth token or
session-cookie exposure, and update or signature-verification bypasses.

## Supported versions

Only the latest release receives security fixes.

## Where the details live

How Boba is designed to resist these problems is documented in [docs/security/](docs/security/).
The release signing and update-verification model is in
[docs/install/updates.md](docs/install/updates.md).
