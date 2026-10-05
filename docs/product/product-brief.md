---
title: Boba Job Find — Product Brief
status: draft
created: 2026-10-04
updated: 2026-10-04
owner: pm (John)
inputs:
  - AGENTS.md (locked product decisions, 2026-10-04)
  - docs/research/upstream-analysis.md
  - docs/research/competitors.md
  - docs/research/integrations.md
  - docs/research/offer-scoring.md
  - ai-job-search-manual.md (upstream manual)
---

# Product Brief: Boba Job Find

> A calm, honest job-search coach that runs on your own computer. It finds offers every day, tells you which ones are worth your time and why, writes truthful tailored applications, and sends them only as far as you allow.

## 1. Executive summary

Job seekers who apply across borders lose hours every week to the same loop: scrolling many boards, guessing which posts are real and winnable, rewriting a CV and cover letter from scratch, and losing track of who replied. The tools that automate this are cloud SaaS products with weekly billing. They take the user's whole work history and often send generic applications at scale, which get rejected at scale.

**Boba Job Find** is a free, open-source, local-first desktop app (Windows, macOS, Linux). It turns the proven [`ai-job-search`](https://github.com/MadsLorentzen/ai-job-search) methodology into a product that a person can install and use without cloning a repo, installing LaTeX or writing prompts. These are the methodology pieces it keeps:

- hard eligibility gates
- transparent fit scoring
- a rule against fabricated claims
- mechanically checked, ATS-safe documents

Boba Job Find adds:

- a first-run wizard
- a daily search that runs from the system tray
- a ranked inbox that shows two scores, Fit and Quality
- a per-application tailoring studio
- an application tracker that updates itself from recruiter emails

Users can apply with a review step before each send, or turn on bounded auto-apply for the channels where it is allowed.

The AI runs on the agent CLI the user already has (Claude Code, Codex, OpenCode, Gemini CLI, Aider…). Boba ships a specialised job-search agent that runs on top of that CLI. There is no Boba account, no Boba server and no Boba bill.

**Why now:** capable agent CLIs are now installed on many developer machines, including local-model routes through OpenCode/Aider + Ollama. Job boards are closing their read APIs, while public ATS job boards (Greenhouse, Lever, Ashby…) stay open. Ghost jobs are well documented: 27.6% of open postings are more than 90 days old (Unlisted, Sep 2026). As a result, deciding *which* jobs deserve an application is now worth more than sending a large number of applications.

## 2. The problem

**Primary persona: Diego**, a LATAM software developer. He is fluent in Spanish and English, applies for remote roles worldwide and also looks at LATAM and Spain. He is the first user and the person building the product.

His typical week:
- **Discovery is scattered.** He checks 6–10 sources: remote boards, LinkedIn, company career pages, regional boards such as Computrabajo. The same job appears three times, and some posts are months old.
- **Triage is guesswork.** He can't tell quickly whether he's eligible ("US only"? "must be EU resident"?), whether the pay clears his floor, or whether the company is still hiring.
- **Tailoring is expensive.** A good application takes 45–90 minutes: rewrite the CV, write the letter, and decide whether it should be in ES or EN. Generic applications get no replies. The agent-repo workflow fixes the quality but needs a terminal, Bun, Python and LaTeX.
- **Follow-through leaks.** Replies end up in his inbox among other mail. He forgets who to follow up with. Rejections arrive without context, so he never learns what worked.
- **It hurts.** Job searching is stressful and lowers self-esteem. Spam-apply tools make this worse with "500 applications sent, 2 replies".

[ASSUMPTION] These baseline numbers are estimates and should be checked with Diego: about 6 h/week on discovery and triage, about 60 min per tailored application, and a reply rate below 10% on generic applications.

**How people cope today:** manual spreadsheets, Teal or Huntr for tracking, Jobscan for keywords, LazyApply or Sonara for volume. All of these run in the cloud, most are subscriptions, none explain *why* a job ranked high, and none run on Linux or offline.

## 3. The solution (the experience)

1. **Ten-minute setup.** The wizard detects installed agent CLIs. The user picks one and a model, imports a master CV (PDF/DOCX/LinkedIn export) into a structured profile, sets preferences (roles, salary floor, locations, remote, languages), connects sources and, optionally, an email account.
2. **Every morning, a short ranked list.** A tray job runs the daily search. A native notification shows something like: "3 strong matches today 🧋". New offers are fetched, duplicates removed, eligibility gates applied, and each offer is scored on **Fit** (can I get it?) and **Quality** (do I want it?). Each score has evidence and a clear list of gaps.
3. **Read, decide, tailor.** The offer detail screen explains the score in plain language: what matched, what is missing and what looks off (stale post, no salary, layoffs). One click produces a tailored CV and cover letter in the offer's language, using only facts from the profile. Every claim can be traced back to the profile, and the PDF is checked to confirm an ATS can read it.
4. **Apply the way the user wants.** In review mode, Boba prepares everything and the user sends it. In auto mode, which is opt-in per source and channel, Boba sends applications that clear the user's score threshold. Daily caps, a kill switch and a full audit log apply.
5. **The tracker keeps itself current.** A kanban board (Saved → Applied → Interviewing → Offer / Rejected). Recruiter replies are read locally and *proposed* as status changes. Nudges suggest follow-ups the user can send.

Tone throughout: an **encouraging coach**. It is warm and celebrates progress, and it is honest when a job is a reach.

## 4. What makes this different (honestly)

| Every incumbent | Boba Job Find |
|---|---|
| Cloud SaaS holds your CV and history | **Local-only, encrypted** store; no vendor backend; export anytime |
| Bundled AI, weekly or credit billing | **Free & open source**; uses the agent CLI and model you already have |
| Black-box match % | **Two transparent scores** with evidence, gaps and confidence; weights you can edit |
| Volume auto-apply that reads identically | **Quality-gated** apply: tailored per offer, threshold and caps, human-in-control by default |
| Fabricated or "keyword-stuffed" CV lines | **No-fabrication guardrail**: claims come only from your profile; gaps are declared, not stuffed |
| English-first, US-first | **Bilingual ES/EN** UI and documents, LATAM + Europe + global remote |
| Web + Chrome only | **Native Windows, macOS and Linux** |

**What we will not beat incumbents on:** coverage of ATS autofill across hundreds of sites, listing freshness at aggregator scale, and zero-setup onboarding for non-technical users. Our advantage is the private, judgement-heavy layer (ranking, tailoring, honest coaching), not volume. Being free and open source also means no growth team and no paid data. That is accepted.

## 5. Who this serves

- **Primary (MVP): bilingual tech job seekers applying across borders.** Typically LATAM developers targeting global remote, US/Canada, Europe or LATAM roles. They are comfortable installing a desktop app and already have, or can install, an agent CLI. Diego is the first of them.
- **Secondary:** privacy-sensitive professionals who won't upload a CV to an unknown cloud, Linux and offline users, and power appliers who want to control costs.
- **Not for (v1):** recruiters or agencies applying on behalf of others, people who want mass-apply volume, users without any agent CLI and unwilling to install one, and mobile-only users.

## 6. Product principles

1. **Local-first, always.** User data never leaves the machine unless the user triggers it: a fetch, a send, or a call to their chosen agent CLI. No telemetry carrying personal data. No Boba server.
2. **Honest coach.** Be encouraging about the person and truthful about the job. Never invent experience. Show gaps and confidence, and present a high score with low confidence as a lead, not a verdict.
3. **Human in control.** Nothing irreversible happens silently. Auto-apply is opt-in, bounded, logged and stoppable with one click. Status changes inferred from email are proposals.
4. **Transparent over clever.** The core score is deterministic and explainable. The LLM extracts information and explains the score; it never quietly changes a number.
5. **Respect the platforms.** Sanctioned and public sources by default. Anything that may breach a site's terms is opt-in per source, with the risk stated plainly.
6. **Calm by design.** Boba aesthetic, short daily digests, no streaks or guilt. Less noise is a feature.
7. **Bilingual by default.** ES and EN are equal everywhere: UI, profile and generated documents.

## 7. Success criteria

The product is free and for personal use, so success is measured by **Diego's outcomes and habit**, plus community adoption later. The app tracks these metrics locally only, with no telemetry.

| # | Metric | Target |
|---|---|---|
| S1 | Time from install to first ranked offer list | ≤ 15 min |
| S2 | Daily triage time (open digest → decisions made) | ≤ 10 min/day |
| S3 | Time to produce a reviewed, tailored application | ≤ 10 min (vs ~60 min baseline) |
| S4 | Reply rate (any human response) on Boba applications | ≥ 2× the user's own pre-Boba baseline over 30 applications |
| S5 | Interview rate per application | ≥ 15% over 30 applications [ASSUMPTION: from upstream's 20/69 anecdote, discounted] |
| S6 | Fabricated claims in sent documents | **0**: every claim traces to the profile |
| S7 | Unintended submissions (sent without matching a user rule or approval) | **0** |
| S8 | Generated PDFs passing ATS text-layer verification | 100% |
| S9 | Retention: Diego still runs the daily search after 30 days | Yes |
| S10 | Community (post-MVP) | 100 GitHub stars and 10 external contributors in 6 months after public release [ASSUMPTION] |

**Counter-metrics (do not optimise):** number of applications sent, which would reward spam; time spent in the app, which would reward engagement over outcomes; and average score, which would encourage inflated scoring.

## 8. Scope

**MVP:**
- onboarding wizard with agent-CLI detection
- profile import
- safe sources: ATS public boards, remote feeds, Adzuna/Jooble, manual URL or text paste, custom RSS and ATS board
- daily tray search with notifications
- deduplication
- Fit and Quality ranking with gates and explanations
- tailoring studio (ATS PDF and DOCX, 2+ templates, ES/EN)
- review-mode apply
- bounded auto-apply for email-apply and public ATS forms
- opt-in LinkedIn and Indeed session sources with per-source opt-in auto-apply (ToS/ban warning, caps, kill switch, audit log)
- kanban tracker
- email connection: send, plus local reading of replies that produces proposed status updates
- encrypted local store
- export and backup
- EN/ES UI, light and dark themes

**v1.1:**
- LATAM session sources (Computrabajo, OCC, Bumeran)
- InfoJobs sanctioned apply
- interview prep pack
- follow-up drafting
- calibration of score weights from outcomes
- custom source adapter SDK
- more templates

**Later:**
- mock interviews
- skill-gap and upskill plans
- salary negotiation helper
- opt-in end-to-end encrypted sync between the user's devices
- more UI languages

**Explicitly out:** accounts, billing, a cloud backend, mobile apps, recruiter-side features and mass-apply as a goal.

## 9. Key risks & open questions

1. **Auto-apply reality.** Research shows that sanctioned candidate auto-submit exists only through InfoJobs. ATS apply APIs need an employer key, and LinkedIn, Indeed and WeWorkRemotely forbid automated submission. *Policy (orquestador, 2026-10-04):* full auto-apply is available.
- **Auto-capable by default:** email-apply, public ATS forms and sanctioned APIs (InfoJobs, v1.1).
- **Behind an explicit per-source opt-in:** LinkedIn, Indeed and similar. The opt-in comes with an account-ban/ToS warning, conservative caps and pacing, an automatic pause on challenges and the global kill switch.
- **Diego decided on 2026-10-04:** session-board auto-apply ships in MVP. The core executes it, never LLM tools. The warning copy and default limits go to reviewer audit.
2. **Reading Gmail.** Reading recruiter replies uses a *restricted* Gmail scope, which normally requires a CASA assessment. Proposed for a free OSS app: **bring your own OAuth client** (the user creates their own Google Cloud client, a guided 5-minute step) or use IMAP with an app password. Outlook through Microsoft Graph delegated scopes works without admin consent. **Diego decided on 2026-10-04:** bundled public desktop OAuth clients plus a BYO fallback. Gmail is send-only by default; reading uses Graph, IMAP or a BYO client; CASA is deferred.
3. **Agent CLI variance.** Every CLI has different headless flags, output formats and rate limits. Where non-interactive JSON output is missing, support may be uneven. The architect must define an adapter contract and a supported-CLI matrix.
4. **Token and quota cost of the daily run.** Scoring every new offer with an LLM could use up a subscription's limits. Mitigation: deterministic pre-filter, LLM only on candidates that pass it, and a per-run budget.
5. **Install friction.** The user still needs an agent CLI installed. Boba detects missing ones and links to install guides, but does not bundle them.
6. **Legal.** Sources driven by browser automation carry a risk of account bans. They stay opt-in, with a stated warning and no default-on.

---
*Downstream: PRD → `docs/product/PRD.md`; epics → `docs/product/epics-and-stories.md` (after architecture).*
