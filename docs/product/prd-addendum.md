---
title: Boba Job Find — PRD Addendum
status: draft
created: 2026-10-04
updated: 2026-10-04
owner: pm (John)
---

# PRD Addendum

This addendum holds technical detail that sits behind the product contract in `PRD.md`. The architect owns the final calls on everything here.

## A.1 Agent CLI matrix (verified by architect, 2026-10-04)

The architect checked these flags locally against Claude Code 2.1.289 and OpenCode 1.18.34. Codex and Gemini were checked against vendor documentation.

**Streaming output is required.** Drivers use streaming output, not plain JSON, so that progress can be shown and runs can be cancelled.

**Tiers are probed when Boba detects the CLI** (AD-4); the tier shown below is indicative only. A CLI without verified `tool_lockdown` cannot run untrusted-content tasks at all (`extract_postings`, `narrate_scores`, `tailor_application`, `review_application`, `classify_emails`); it runs only `profile_import` and `translate_profile` (ADR-010, hard gate, no override).

| CLI | Non-interactive invocation | MCP / tool lockdown | Structured output | Indicative tier |
|---|---|---|---|---|
| Claude Code | `claude -p --output-format stream-json --verbose --model <m> --max-turns <n> --max-budget-usd <x> --mcp-config <f> --strict-mcp-config --allowedTools 'mcp__boba__*' --no-session-persistence` | `--strict-mcp-config` + allowlist | `--json-schema` | Full |
| Codex CLI | `codex exec --json --ephemeral --skip-git-repo-check -m <model> --sandbox read-only --output-schema <f>` | MCP via `-c mcp_servers.*` + `--ignore-user-config` | `--output-schema` | Full / Limited (its shell can still read the workspace) |
| Gemini CLI | `gemini -p <prompt> --output-format stream-json -m <model>` | MCP + `excludeTools` in the workspace `.gemini/settings.json` | stream-json | Full |
| OpenCode | `opencode run --format json -m provider/model` (`opencode models` lists the models) | MCP + permissions in the workspace `opencode.json` | json | Full |
| Aider | `aider --message <p> --yes-always --no-git --model <m>` | none unless a runtime MCP probe passes | text | Limited (text-only) |

**Errors (FR-21).** Each driver detects auth and quota errors from the exit code plus stderr or event patterns. They are normalised to `Failed{AuthRequired | QuotaExceeded | LimitExceeded}`.

## A.2 MVP Safe Source set

The list is based on `docs/research/integrations.md` §7 Tier 0.

| Source | Type | Auth | Market | Notes |
|---|---|---|---|---|
| Greenhouse boards | ATS Board | none | any | User adds a company token; can also be pre-seeded with a curated list [v1.1] |
| Lever postings | ATS Board | none | any | EU endpoint is `api.eu.lever.co` |
| Ashby job boards | ATS Board | none | any | `includeCompensation=true` returns structured salary |
| Workable widget | ATS Board | none | any | Public but undocumented, so it may break |
| SmartRecruiters | ATS Board | none | any | Public GET; no contract |
| Remotive | Feed/API | none | global remote | Max 4 fetches/day; attribution required |
| RemoteOK | Feed/API | none | global remote | Returns the newest 100 only; a backlink is required |
| Himalayas | Feed/API | none | global remote | Max 20 per page; attribution required |
| Arbeitnow | Feed/API | none | Europe | — |
| WeWorkRemotely | RSS | none | global remote | **Auto-apply disallowed**: Manual channel only |
| HN Who's Hiring | Algolia API | none | global/tech | Comments need LLM extraction |
| Adzuna | API | user app_id/key | 20+ countries incl. BR, MX | 250 calls/day; salary data |
| Jooble | API | user key | 60+ countries incl. LATAM | Only 500 lifetime requests per key; use sparingly |
| Manual paste/URL | Manual | — | any | The covered path for LinkedIn, Indeed, Computrabajo… |

**Gap:** LATAM-specific sources are weak in the Safe tier. Adzuna and Jooble give partial coverage. LinkedIn and Indeed are Session Board Sources in MVP (opt-in). Computrabajo, Bumeran and OCC follow in v1.1; until then, manual paste covers them.

## A.3 Apply Channel matrix

| Channel | Review mode | Auto mode | Phase | Mechanism (indicative) |
|---|---|---|---|---|
| Email Apply | ✅ | ✅ (rules) | MVP | Sent from the user's connected mailbox, with PDFs attached |
| ATS Form: Greenhouse, Lever, Ashby, Workable hosted | ✅ (fill, user submits or confirms) | ✅ (rules; stops on CAPTCHA or login) | MVP | Automated browser on the employer's public form |
| Manual | ✅ (copy kit + mark applied) | ❌ | MVP | — |
| InfoJobs API | ✅ | ✅ | v1.1 | Candidate OAuth with the sanctioned `POST /offer/{id}/application` |
| Session Board (LinkedIn Easy Apply, Indeed, Computrabajo…) | ✅ (fill; user clicks submit) | ⚠ **per-Source opt-in only**: ToS/ban warning, 3/day default cap (max 10), ≥10 min pacing, ≥60 min Outbox hold, 7-day auto-pause on challenge (FR-91) | **MVP** (LinkedIn, Indeed); LATAM boards v1.1 | Automated browser in the user's own session |
| WeWorkRemotely | Manual only | ❌ (site disallows applying outside its own site) | MVP | — |

**ATS Form mechanism.** chromiumoxide drives the user's own Chrome or Edge in a dedicated Boba profile, in a headed (visible) window. It stops on a CAPTCHA or login wall and never solves CAPTCHAs. `session_form` is the `apply_capability` for LinkedIn/Indeed-type sites, whose limits are in FR-91. The architecture adopts those limits, plus a 30-day re-confirmation.

**Risk to check for ATS Form auto-submit:** ATS forms are public, but some employers add CAPTCHAs or custom questions. Expect a real share of these sends to fall back to the review queue. That fallback is the intended behaviour, not a failure.

## A.4 Storage, agent sandbox and adapter trust (architect validation)

- **Storage (FR-109).** Documents, attachments and email content are stored as BLOBs **inside** the SQLCipher database (rusqlite with SQLCipher 4). The key lives in the OS keychain via keyring-core. On Linux without Secret Service, the fallback is an Argon2id passphrase. The app data directory holds only:
  - the encrypted DB
  - redacted logs
  - temporary agent workspaces, deleted after each run
  - the browser profile

  The browser profile's cookies are protected by **Chrome's own OS-level encryption, not Boba's**. The ToS warning for Session Board Sources (FR-30) must say so. Pending Diego's approval (architecture §17 Q6).
- **Agent sandbox (NFR-7).** The agent has **no shell and no file access**. Its only tools are Boba MCP tools (read, propose, save draft); none of them sends or submits. These are served by the `boba-mcp` stdio shim with a per-run token and a per-task tool subset. Each run gets a fresh temporary workspace with limits on turns, tool calls, time and USD (AD-6, ADR-002).
- **External-process adapters (FR-29, v1.1).** These are **not OS-sandboxed**, and the warning says so honestly. The safeguards are:
  - trust is pinned to the path plus a blake3 hash, so the user must re-approve when the binary changes
  - a minimal environment with no secrets, and a temporary working directory
  - a supervisor timeout and kill-tree
  - stdout is capped, checked against a schema and treated as untrusted
  - no access to the DB, the keychain or MCP

## A.5 Scoring reference

The scoring factor definitions, default weights, ghost-job signals and worked example all live in `docs/research/offer-scoring.md`. The PRD adopts it as the default model. One change for MVP: Q2 Company signals is limited to signals found in the posting plus cached research.

## A.6 Upstream reuse map (summary)

| Upstream piece | PRD home |
|---|---|
| `04-job-evaluation.md` gates and rubric | FR-53 to FR-57 |
| `verify_pdf.py` / `verify_layout.py` | FR-76, FR-77 |
| Relevance-weighted cutting | FR-76 |
| Reviewer pass | FR-22, FR-72 |
| Untrusted posting boundary | FR-19, NFR-4 |
| "Profile, not CV" no-fabrication rule | FR-73, FR-74 |
| `job_key.py` dedup | FR-49 |
| `robots_check.py` | FR-32 |
| Tracker status vocabulary | FR-93, FR-110 (export compatibility) |
| `/gmail-sync` propose-then-approve | FR-102 |
| `/outcome followup` (draft, never send) | FR-96 |

Upstream code is MIT, so its notice must be kept (NFR-20).
