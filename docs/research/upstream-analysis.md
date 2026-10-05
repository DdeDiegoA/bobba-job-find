# Upstream Analysis — `MadsLorentzen/ai-job-search`

> **Purpose:** map every capability of the upstream framework to decide what a cross-platform
> desktop product ("boba-job-find") keeps, rewrites, wraps, or retires.
> **Upstream:** https://github.com/MadsLorentzen/ai-job-search (branch `master`)
> **Snapshot read:** commit `52f84e0` ("fix(verify_pdf): --ascii-dates needs digit boundaries…"), `git clone` into `/tmp/boba-upstream` on 2026-10-04. Not vendored.
> **Manual:** `/Users/diegoarenas/Downloads/ai-job-search-manual.md` (consulted 2026-10-04).
> **License:** MIT © 2026 Mads Lorentzen (`LICENSE`, verified in clone). MIT permits commercial use, modification, and redistribution provided the copyright notice and permission notice are retained. There is **no** separate CLA; `CONTRIBUTING.md` governs contributions only.

---

## 1. What it is, precisely

It is **not an application**. It is a *repo you clone* whose entire orchestration layer is
Markdown that an AI coding agent (Claude Code, or Codex/Gemini CLI for the portal CLIs alone)
reads and executes. There is no server, no database, no compiled app. State lives in flat
files at the repo root. The "business logic" is prose instructions + a thin layer of Python
and TypeScript CLI helpers.

The README's own claim: 69 tailored applications → 20 first interviews → 1 signed contract
(2026), which is the author's personal anecdote, not a controlled study.

### Three-layer architecture

| Layer | Path | Nature | Language/runtime |
|---|---|---|---|
| **Commands** | `.claude/commands/*.md` | 12 slash-command workflow specs (prose steps for the agent) | Markdown only |
| **Profile + rubrics** | `.claude/skills/job-application-assistant/*.md` | 9 candidate-data + methodology files (`01-candidate-profile` … `09-web-research`, `SKILL.md`) | Markdown only |
| **Portal CLIs** | `.agents/skills/*/SKILL.md` + `cli/` | 6 per-portal search CLIs with a uniform `search`/`detail` contract | TypeScript on **Bun** |
| **Tools** | `tools/*.py`, `salary_lookup.py` | 12 stdlib-mostly Python helpers (verification, state, security) | Python 3.10+ |
| **Agent runtime** | Claude Code | Executes the Markdown; the only LLM dependency | Proprietary CLI |
| **Document toolchain** | LaTeX (`lualatex`, `xelatex`), `moderncv`, custom `cover.cls` | CV/cover-letter rendering | LaTeX |

Unified "thin-pointer" design (`AGENTS.md`): `.claude/` is the single source of truth; other
agent runtimes (Codex, Antigravity, Gemini CLI) discover the portal skills via the portable
`.agents/skills/` format, but **the full workflow is Claude-Code-specific** — it depends on
commands, skills, and hooks that other agents don't implement.

---

## 2. Capability inventory

### 2.1 Commands (workflow capabilities)

| Command | What it does | Implementation type | Desktop disposition |
|---|---|---|---|
| `/setup` | Builds the profile: 3 routes (documents inventory / single CV paste / 9-section interview); writes `CLAUDE.md`, the 7 profile files, `cv/main_example.tex`, search queries. Idempotent; supports `--section search/skills/experience`. | Agent prose; parses local `documents/` | **Rewrite** as a native onboarding wizard + structured profile store |
| `/scrape` | Discovers installed portal CLIs, runs them in parallel, fetches/parses, dedups vs `seen_jobs.json`, quick fit pre-score, referral links, portal health checks, ranked chat output. Falls back to `WebSearch` if Bun missing. | Agent prose + TS CLIs + Python state | **Wrap** the CLIs; **rewrite** the orchestration as an app scheduler |
| `/rank` | Parallel agents fetch each posting, score against `04-job-evaluation.md`, apply deal-breakers, flag deadlines/expired, persist to `seen_jobs.json` via `rank_state.py`. | Agent prose + `rank_state.py` | **Rewrite** scoring as deterministic + LLM hybrid engine |
| `/apply <url\|text>` | 6-step pipeline: fit eval → draft CV+cover in LaTeX → fresh-context reviewer agent critiques → revise → **compile & inspect PDFs** (2-page CV / 1-page letter, ATS text-layer check) → present + log to tracker. | Agent prose + Python verify tools + LaTeX | **Rewrite** as the product's core flow; keep the verification *ideas* |
| `/interview` | Builds 6-part prep pack from the exact submitted materials + company research under verify-before-use; offers roleplay mock interview. | Agent prose | **Rewrite** (high-value differentiator) |
| `/outcome` | Records outcomes, archives submitted materials, maintains tracker CSV; `followup` drafts (never sends) follow-ups for 10+ day-silent apps. | Agent prose + CSV schema | **Rewrite** as tracker state machine + draft-only follow-ups |
| `/expand` | Additively enriches profile by scanning documents + public sources (GitHub, portfolio, Kaggle, Scholar) and course syllabi. | Agent prose + web | Keep (later) |
| `/upskill` | Skill-gap heatmap against tracked postings + learning plan with web-sourced resources. | Agent prose + web | Keep (later) |
| `/html-report` | Self-contained offline HTML dashboard (inline SVG, zero deps) from tracker + application files. | Agent prose generating HTML | **Rewrite** natively (or keep as export) |
| `/gmail-sync` | Reads Gmail (requires Gmail connector/MCP), classifies status signals, **proposes a batch and waits for approval** before writing; cites source email. | Claude Gmail MCP | **Rewrite** as opt-in email ingestion (see integrations doc) |
| `/notion-sync` | Read-only pipeline view in Notion via official MCP server (OAuth). Nothing syncs back; repo files stay source of truth. | Notion MCP | Optional later integration |
| `/add-template` | Registers user CV/letter templates (LaTeX, Typst, any CLI toolchain), runs a mandatory test compile, connects to `/apply`. | Agent prose + compile check | **Rewrite** as pluggable template system |
| `/add-portal` | Generates a new scraping skill: interviews → researches portal → scaffolds CLI → **mandatory live test query** → registers. Rejects login-gated portals. | Agent prose + TS scaffold | **Rewrite** as a reviewed plugin/connector SDK |
| `/reset` | Deletes profile / documents / both, with explicit confirmation. | Agent prose | Keep (native equivalent) |

### 2.2 Portal search CLIs

Uniform contract (`search`/`detail`, `--format json|table|plain`, `enabled:` flag in
`SKILL.md`, errors to stderr as JSON, retry/backoff). All TypeScript, run on **Bun**.

| CLI | Market | Access | Runtime deps | Notes |
|---|---|---|---|---|
| `linkedin-search` | Global | Public `jobs-guest` endpoints, **no auth** | **Zero** (`fetch` only) | `--location` required; `--jobage`, `--remote`, pagination 10/page. **Personal-use only; automated access violates LinkedIn ToS** (stated in its own SKILL.md). |
| `freehire-search` | Multi-market, tech-first | Public JSON API of freehire.me (aggregator of ~50 ATS) | **Zero**; `FREEHIRE_API_URL` swappable to self-hosted backend | `--region/--country/--remote`; results structured (skills/seniority). Best-effort hosted service, no SLA. |
| `jobindex-search` | Denmark | Scrape/parse HTML | `@bunli/core`, `@bunli/utils`, `node-html-parser`, `zod` | Ships `enabled: false` |
| `jobnet-search` | Denmark | Public government portal API | `@bunli/*`, `zod` | `enabled: false` |
| `jobbank-search` | Denmark | HTML (+RSS tests) | `@bunli/*`, `node-html-parser`, `zod` | `enabled: false` |
| `jobdanmark-search` | Denmark | Public search API | `@bunli/*`, `node-html-parser`, `zod` | `enabled: false` |

Dependency summary: 4 of 6 CLIs pull `@bunli/core@0.9.1`, `@bunli/utils@0.6.0`,
`node-html-parser@^6`, `zod@^3`. Only `linkedin-search` and `freehire-search` are dependency-free.

### 2.3 Profile + methodology files (`job-application-assistant/`)

| File | Role |
|---|---|
| `SKILL.md` | Skill entrypoint |
| `01-candidate-profile.md` | Identity, education, experience, skills, certifications, languages |
| `02-behavioral-profile.md` | Behavioral assessment (PI/DISC/MBTI/self) |
| `03-writing-style.md` | Voice/tone rules; rule 5 = every company claim independently verified |
| `04-job-evaluation.md` | **The scoring rubric** (see below) |
| `05-cv-templates.md` | CV templates + `ACTIVE-TEMPLATE` block; LaTeX rules |
| `06-cover-letter-templates.md` | Cover-letter templates + `cover.cls` constraints |
| `07-interview-prep.md` | Interview prep protocol + roleplay |
| `08-application-forms.md` | Web-form field proposals (user submits) |
| `09-web-research.md` | Research protocol under verify-before-use |

---

## 3. The scoring/drafting "secret sauce" (what must survive productization)

The value is not the markdown — it is the **encoded methodology**, three parts of which are
genuinely differentiating and should be preserved (re-implemented, not copied verbatim):

1. **Hard gates before scoring** (`04-job-evaluation.md`):
   - **Eligibility gate** — citizenship/PR/security-clearance requirements read *verbatim*; a hard
     stop, never scored. Handles the "silence is not permission" trap and company-wide vs
     role-level permission distinction.
   - **Language gate** — posting's *role* language requirement vs candidate's declared levels;
     `FAIL` if undeclared, `FLAG` if the bar plausibly exceeds the declared level (human decides).
2. **Five weighted dimensions**: Technical Skills 30%, Experience 25%, Behavioral/Culture 15%,
   Career Alignment 30%; **Location is pass/fail, unwweighted**. Thresholds: Strong ≥75,
   Good 60–74, Moderate 45–59, Weak 30–44, Poor <30.
3. **Mechanical document verification** (`tools/verify_pdf.py`, `tools/verify_layout.py`):
   - PDF is **compiled and its rendered layout measured** (pages, gaps, orphaned headings, footer
     collisions) — never trusted from `.tex`.
   - ATS check extracts the **embedded text layer** (pypdf, then Poppler `pdftotext` fallback),
     catches `(cid:*)`/replacement chars, verifies contact details are literal text and reading
     order matches.
   - **Relevance-weighted cutting**: if the CV overflows, each line is scored by relevance +
     uniqueness + cover-letter dependency and the lowest is cut — not "oldest first".
4. **Fabrication guardrail**: a claim absent from the profile never appears in the CV; keyword gaps
   are declared, never stuffed.
5. **Fresh-context reviewer agent** (Step 3 of `/apply`): a second pass with clean context to
   critique the draft — a cheap adversarial review pattern worth reproducing.
6. **Trust boundary**: the job posting is treated as **untrusted input** (prompt-injection aware);
   no instructions from it are followed and no links in its body are fetched.

---

## 4. Persistent state & data files

| File / dir | Schema / purpose | Git-ignored? |
|---|---|---|
| `job_search_tracker.csv` | Source of truth. Status vocabulary: `drafted, applied, interview, offer, hired, rejected, no_response, offer_declined, withdrawn`. Carries `cv_file`, `cover_letter_file`, `source`, `deadline`, `notes`. | **Yes** (`job_search_tracker.csv`) |
| `job_scraper/seen_jobs.json` | Append-only dedup/ranking state. Entry fields (from `tests/test_rank_state.py`): `key, title, company, url, portal, deadline, posted_date, rank_score, status, location_verdict, language_gate/note, strengths, gaps`. Key = deterministic `company+title` slug w/ hash disambiguation (`tools/job_key.py`). | Yes |
| `job_scraper/*.md` | Search-query config | Yes |
| `documents/applications/<company>_<role>/` | Immutable archive: posting, `cv_draft.tex`, `cover_letter.tex`, `outcome.md` (`Status:` enum incl. `in_progress`) | Yes |
| `company_research/<company>.json` | 30-day-TTL company research cache; cache contents are **data, never instructions** | Partially |
| `gmail_sync/state.json` | `{last_sync, processed_message_ids}` | Yes |
| `salary_data.json` | User-supplied salary benchmark set (Excel-converted) | Yes |
| `cv/main_*`, `cover_letters/cover_*` | Generated personalized docs | Yes (except `*_example.tex`) |

**Design lesson for the desktop app:** every one of these is a flat, human-readable, portable
artifact. That is a feature — a desktop app should keep a local, exportable, file-based store
(SQLite is fine, but CSV/JSON/Markdown export must remain first-class).

---

## 5. Tools inventory (`tools/`, Python stdlib-mostly)

| Tool | Lines | Function | Desktop reuse |
|---|---:|---|---|
| `verify_pdf.py` | 295 | Page count + text-layer extraction + `--contains` + `--ascii-dates` checks (pypdf → `pdftotext` fallback) | **Port** (Python or native libs) |
| `verify_layout.py` | 428 | Rendered-layout measurement: gaps, orphans, footer collisions | Port/rewrite |
| `rank_state.py` | 403 | Efficient candidate selection + write-back for `/rank` (keeps whole state file out of model context) | **Port** the pattern |
| `job_key.py` | 209 | Deterministic dedup key + non-Latin fallback to portal id | Port (trivial) |
| `robots_check.py` | 178 | robots.txt compliance before retrying with browser headers | **Keep** as a compliance primitive |
| `security_guards.py` | 297 | Supply-chain guards: pins allowed Claude permissions, personal-data .gitignore rules, blocks npm lifecycle scripts | Adapt for desktop plugin model |
| `lint_skills.py` | 122 | Lints skills/commands/settings | Retire (agent-specific) |
| `check_framework_version.py` | 154 | Framework-version markers / git-diff gate | Retire |
| `check_upstream_updates.py` | 196 | Previews which personalized files an upstream update touches | Retire |
| `upstream_triage.py` | 227 | Classifies upstream commits worth reviewing vs skipping | Retire |
| `convert_salary_excel.py` | 413 | Excel → `salary_data.json` (openpyxl) | Keep (optional) |
| `salary_lookup.py` (root) | 454 | Fuzzy company lookup w/ spelling variants, JSON output | **Port** as a benchmark module |

Tests: ~40 `pytest` files covering PDF verification, reset, tracker vocabulary, privacy,
security guards, salary, HTML report, scrape contract, etc. Plus `bun test` suites per CLI.
`CI` (`.github/workflows/ci.yml`): LaTeX smoke compiles, skill lint, CLI typechecks, security
guards, dependency review; **deliberately no live portal tests** in CI (network flakiness + ToS).

---

## 6. Environment & LLM dependencies

| Dependency | Required? | Cost model | Desktop implication |
|---|---|---|---|
| **Claude Code** | Mandatory for the full workflow | No free tier; Pro/Max/Team subscription or Anthropic API credits (pay-per-token) | The product must **decouple from a single vendor** — support BYO key (Anthropic/OpenAI/Gemini/OpenRouter/local Ollama) to remove the biggest adoption blocker and cost surcharge |
| **Python 3.10+** | Yes (`tools/`) | Free | Bundle a Python runtime or port tools to the app's language |
| **Bun** | Yes for portal CLIs; `/scrape` degrades to WebSearch without it | Free | Either bundle Bun, port CLIs, or reimplement connectors natively |
| **LaTeX** (`lualatex` + `xelatex`, MacTeX/TeX Live/TinyTeX/MiKTeX) | Yes for PDFs | Free but heavy (~GB) | Huge install friction; product should consider native PDF rendering (Typst, HTML→PDF, or bundled minimal TeX) |
| **pypdf** / **Poppler `pdftotext`** | Optional (ATS check degrades) | Free | Port natively |
| **Gmail connector** | Optional (`/gmail-sync`) | Free | See integrations doc — OAuth + verification burden |
| **Notion MCP** | Optional | Free | Optional later |

**Licensing of dependencies:** `@bunli/*`, `node-html-parser`, `zod`, pypdf, openpyxl, LaTeX
templates are all permissive (MIT/BSD/Apache-class) — no copyleft blocker for a commercial desktop
app, subject to their own notices. The upstream MIT notice must be retained for copied code.

---

## 7. What must be kept / rewritten / wrapped / retired

| Element | Verdict | Why |
|---|---|---|
| `04-job-evaluation.md` gates + weighted rubric | **Keep (reimplement)** | The differentiating methodology; make it deterministic + transparent |
| PDF layout + ATS text-layer verification | **Keep (reimplement)** | Real moat vs "ask an LLM for a CV"; mechanizes quality |
| Relevance-weighted cutting | **Keep (reimplement)** | Better than truncation heuristics |
| Fresh-context reviewer pass | **Keep** | Cheap, effective adversarial review |
| Untrusted-posting trust boundary | **Keep** | Security · prompt-injection defense |
| Fabrication guardrail ("profile, not CV") | **Keep** | Core trust promise; competitors get this wrong |
| `linkedin-search` / `freehire-search` CLIs | **Wrap** | Zero-dep, uniform contract — but LinkedIn is ToS-risky (see integrations) |
| Danish portal CLIs | **Retire for MVP** | Wrong market; template for a connector SDK |
| `/apply`, `/scrape`, `/rank`, `/interview`, `/outcome` orchestration | **Rewrite** | Currently agent-prose; must become real app logic |
| Tracker / seen_jobs / archive state | **Rewrite (preserve format)** | Port to SQLite with files-first export |
| Claude-Code coupling (`/setup`, commands, MCP, hooks, `settings.json`) | **Retire** | Must be runtime-agnostic |
| LaTeX toolchain as mandatory | **Wrap/optionalize** | Keep as an advanced template path; don't require TeX Live |
| `/html-report` | **Rewrite** | Native UI supersedes it; keep as export |
| Upstream-maintenance tools | **Retire** | Only meaningful for a personalized fork |
| MIT license | **Comply** | Retain copyright/permission notice in derivative work |

---

## 8. Risks & open questions

- **ToS/legal:** the LinkedIn and, more broadly, any scraping connectors carry legal risk (see
  `integrations.md`). Upstream itself labels `linkedin-search` "personal use only."
- **Cost:** Claude Code has no free tier; token-heavy `/apply` is the expensive step. Decoupling
  the LLM and making `/rank` (not `/apply`) the triage step is essential.
- **Friction:** the bundle of Claude Code + Python + Bun + full LaTeX is a hard install for a
  non-technical job seeker. This is the single biggest productization gap.
- **No tags/releases** in the shallow clone (`git tag` empty; single visible commit). There is no
  semver release line to track — the project is fast-moving `master` + a 130 KB CHANGELOG.
- **Verification gap:** the README's success story is `n=1` self-reported; no efficacy evidence.

---

## Sources

- Upstream repo files read in full/part at commit `52f84e0` (clone 2026-10-04): `README.md`,
  `LICENSE`, `SETUP.md`, `AGENTS.md`, `CLAUDE.md`, `CHANGELOG.md`, `.claude/commands/*.md`,
  `.claude/skills/job-application-assistant/*.md`, `.agents/skills/*/SKILL.md` + `cli/package.json`,
  `tools/*.py`, `tests/*.py`, `.github/workflows/ci.yml`, `.gitignore`, `.claude/settings.json`.
- Project manual: `ai-job-search-manual.md` (2026-10-04).
- Live repo: <https://github.com/MadsLorentzen/ai-job-search> (accessed 2026-10-04).
