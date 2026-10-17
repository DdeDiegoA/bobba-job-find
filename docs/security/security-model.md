# Boba Job Find — Security Model

> **Status:** v1, 2026-10-17 · **Owner:** architect · Binding decisions are in [`../architecture/ARCHITECTURE-SPINE.md`](../architecture/ARCHITECTURE-SPINE.md) (AD-n) and the [ADRs](../architecture/adr/README.md). Accepted residual risks are in [`residuals.md`](./residuals.md).

## 1. What we protect

| Asset | Where it lives | Worst case if lost |
|---|---|---|
| Profile, CVs, offers, applications, email metadata | Local SQLCipher DB (AD-2, AD-13) | Private career and contact data exposed |
| OAuth tokens, SMTP/IMAP passwords, DB key | OS keychain (AD-13) | Someone reads or sends the user's mail |
| Board session cookies | A dedicated Chrome profile for Boba, opt-in only (AD-9, S11.1a) | Someone acts as the user on a job board |
| The user's reputation and accounts | Outbound applications, emails, session actions | Spam, bans, wrong applications |
| The updater signing key | CI secrets (AD-22) | A malicious update reaches every user |

The app is local-first with **no server and no account**. There is no cloud component to attack. The attack surface is: untrusted text coming in (postings, emails, pages), the agent CLIs we spawn, the local machine, and our own release pipeline.

## 2. Threat model

| # | Threat | Main defence |
|---|---|---|
| T1 | **Prompt injection.** A posting or email carries instructions that steer the agent (exfiltrate the profile, send an application). | §3 and §4: the LLM proposes, the core disposes; per-run lockdown gate |
| T2 | **Agent output as an attack path.** Injected text returns in narration, drafts or evidence and reaches the UI, Typst or a later run. | ADR-010 §3: output is stored as untrusted, rendered as text, passed to Typst only as JSON data, re-wrapped in the envelope |
| T3 | **Runaway automation.** Auto-apply fires outside what the user configured, too often, or after they pulled the brake. | §6 and §7: `decide()`, caps, spacing, kill switch |
| T4 | **Data at rest.** Another user or a stolen disk reads the DB. | §5: SQLCipher, key in the OS keychain, passphrase fallback |
| T5 | **Token theft or redirect hijack in OAuth.** | §8: loopback, PKCE S256, single-use state, no deep link |
| T6 | **Hidden network activity.** The app talks to a host the user does not expect. | §9: one outbound policy, an observed network log |
| T7 | **Malicious update or poisoned release.** | §10: minisign-verified updates, signed builds, tag-driven CI |
| T8 | **SSRF through custom sources.** A user-added URL points at localhost or the LAN. | AD-10: private, loopback and link-local addresses rejected at connect time (not IP-pinned, so DNS rebinding stays theoretical: R-S4.8-2); redirects off |
| T9 | **Board bans and ToS risk** from session automation. | AD-9 and AD-11: opt-in with warning, strict caps, kill switch, audit |

**Out of scope.** An attacker who already runs code as the same OS user can read process memory and the keychain after unlock. We reduce what a casual local reader gets; we do not defend against a compromised user session (ADR-009, ADR-013).

## 3. The LLM proposes, the core disposes (AD-5)

The agent CLI is the least trusted component. It touches the app only through the `boba-mcp` stdio shim (AD-6), and only with tools that **propose**:

- Agent output is stored as a proposal or draft with provenance `{agent_run_id, cli, model}`.
- The agent never writes a score, a status change, a submission or a setting. There is **no MCP tool** for submitting, sending, deleting or changing settings.
- Scores come only from `boba_domain::scoring`, which is deterministic and offline.
- Tools that save agent output bind `app_id` to the run's own application. The agent's argument is not trusted (S7.8).
- A send is gated **server-side**: the core requires `Ready` (`validate().is_ready`) and a verified document (S7.6) in the send commands (S9.2, S9.3). The UI gate is only a convenience. Exporting an unverified draft to a local file is allowed with a "not verified" warning, because the user chose it; sending is what is blocked.
- Every send path also passes the duplicate guard (S9.5).

## 4. Untrusted content and the lockdown gate (AD-7, ADR-010, ADR-014)

Postings, emails and scraped pages are untrusted. They are wrapped in an `untrusted_content` envelope, but a prompt-level guard does not remove capability, so the real control is **removing the capability**:

1. A task that reads untrusted content (`extract_postings`, `narrate_scores`, `tailor_application`, `review_application`, `classify_emails`) needs a CLI with verified `tool_lockdown`. This is a hard gate with no user override.
2. The gate is checked **per run**: the CLI version must have a pinned lockdown config, **and** the run's own init event must list only `mcp__boba__*` tools. Anything else aborts the run as `Unsupported` before any tool call. A missing init event fails closed.
3. Claude Code runs with `--strict-mcp-config --tools "" --allowedTools mcp__boba__*`. Aider has no MCP and is refused. Codex and OpenCode have configs but only count when their init event is verifiable.
4. Tasks whose input is the user's own (`profile_import`, `translate_profile`) may run on Limited CLIs with a security badge. In **Limited mode** (S3.3) the UI says what is off and why.
5. The agent's workspace is a fresh directory per run, deleted when the run ends. A startup sweep removes leftovers.
6. The agent socket is owner-only: a `0700` directory on Unix, a protected owner-only DACL on Windows. Each run authenticates with its own token.
7. Untrusted scraped payloads are normalised with `ammonia` before they reach the UI.

## 5. Storage and secrets (AD-2, AD-13, ADR-004, ADR-009, ADR-013)

- **One data owner.** Only `boba-store`, in the core process, opens the DB. The UI, `boba-mcp`, agent CLIs and adapters go through core use cases.
- **SQLCipher** encrypts the DB. The key lives in the OS keychain (`keyring-core`).
- **Key safety across updates.** Every macOS build is signed with **one persistent self-signed identity** with a pinned designated requirement, so the Keychain item survives updates (ADR-009).
- **Passphrase fallback** where no keychain is available: an Argon2id KDF record, unlock once per login, zeroized session key and an idle timeout (ADR-013). While **Locked**, scheduled jobs are recorded as `skipped_locked`, one notification is sent per slot, and no job is marked as succeeded. Commands return `errors.store.locked` through the shared port helper (ADR-012).
- **Audit log** (AD-12, ADR-011): append-only, hash-chained, with SQL triggers that reject UPDATE and DELETE. It stores metadata and payload hashes, never payloads. The head hash is anchored in the keychain and checked at startup.
- **Export, backup, reset.** Backups are encrypted and restores are validated before use (S10.2). Export carries no secrets (S10.1). **Reset** wipes the DB, snapshots, browser sessions and keychain entries behind a typed confirmation.
- **Retention.** A sweep removes old documents and mail; the audit chain is untouched (S10.3).
- **Test hygiene.** Tests use a mock keychain and temp directories and never touch real user data.

## 6. Apply policy: `decide()`, caps and spacing (AD-11)

`boba_domain::apply::decide` is the single policy function. The core calls it when an attempt is created, when the Outbox releases it, and right before submit. The UI only shows the `Decision` and its reasons. The core rejects out-of-range settings.

An application auto-sends only if score and confidence pass, no gate returned FLAG or VETO, documents are verified, the global and per-source caps (counted per **local day**) are not exhausted, spacing is respected, and the kill switch is off. A fraud badge blocks Auto on every `decide()` path (S6.8). Everything else lands in the review queue.

**Session boards** (LinkedIn-style, user's own Chrome) have the strictest limits (S11.1a, S11.1b, S11.2, S11.3), all enforced in the core:

| Control | Value |
|---|---|
| Opt-in | Per source, typed confirmation phrase plus warning version, recorded in the audit log; weak-keyring extra acknowledgement on Linux |
| Daily cap | 3 per day by default, hard ceiling 10 |
| Spacing | 10 minutes between auto session applies; 30 s page pacing with jitter (effective floor 18 s) |
| Outbox hold | 60 minutes before release |
| Challenge (CAPTCHA, 429, sign-in wall) | Pauses the source for 7 days |
| Re-confirmation | Every 30 days |
| Execution | By the core, never by an LLM tool |

In this build the browser port never presses a board's submit button for Auto; Auto items park as `channel_unavailable` (residual R-S11.3-1). Review-mode assist fills a form and leaves the submit to the user.

## 7. Kill switch (S9.8)

One control, always reachable: the tray, the header chip and the global shortcut Cmd/Ctrl+Shift+K (engage-only). Engaging it audits the intent first, moves every live Outbox item to the review queue, and stops all auto-send paths. The engine re-checks it **immediately before every irreversible step**. It stays engaged until the user turns it off.

## 8. OAuth and email (S8.1, S8.2, S8.5; spine §11)

- Loopback redirect only, PKCE S256, a single-use 256-bit `state`, a 10-minute TTL. No deep-link fallback.
- Bundled public desktop clients plus a bring-your-own client id. Tokens go to the keychain; refresh tokens support the daily run (`offline_access` for Graph).
- Gmail is **send-only** (`gmail.send`) in the bundled client. Reading uses Graph, IMAP or a BYO Gmail client with `gmail.readonly`, because the restricted scope needs Google verification (CASA, deferred).
- Email reads are read-only. The Graph and IMAP readers and the sink never send.

## 9. Network policy and the network log (AD-10, S10.4)

All outbound HTTP goes through one hardened `HttpClient`, with one exception (below) with https-only apply URLs, redirects off, private-address rejection and secret-free error messages. **Exception:** the optional embeddings model download uses `hf-hub` (fastembed's own client, `boba-embed/src/encoder.rs`) behind the `embeddings` feature, so the `HttpClient` guards do not apply to it. It is opt-in, checksum-verified before the model loads, and recorded in the network log (residual R-S6.9-1). Every request is recorded in the **network activity** view from **all** choke points, including the embeddings model download, so the user can see what the app contacted. Embeddings are opt-in; the model download shows its checksum step and real size (487 MB).

## 10. Releases and updates (AD-22, ADR-009, S10.5)

- The updater is opt-in. A downloaded bundle is **verified with the project's minisign public key before extraction**. Only a bundle signed by the project key reaches the zip extractor, which is why the zip advisories in [`residuals.md`](./residuals.md) are accepted.
- macOS builds use the persistent self-signed identity. Windows uses SignPath Foundation (ADR-009).
- Releases are tag-driven CI. Signing secrets are kept out of pull-request runs (S10.5 F1, fixed). Keys live under `.hermad/keys/` (gitignored) until the maintainer moves them to CI secrets and an offline backup.
- Supply chain: `cargo deny` (advisories, bans, licences, sources) and a licence check run on every change. Every advisory ignore is a recorded security decision.

## 11. Security gates in the build process

Slices that touch unlock, audit integrity, the MCP sandbox, OAuth, signing and session automation went through a **human security gate**, and every advisory ignore or crypto change did too. Each reviewer ran the full CI gate including `cargo deny check`. The reviews are in [`../reviews/`](../reviews/).
