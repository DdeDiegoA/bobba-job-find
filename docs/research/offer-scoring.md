# Offer Scoring — A Transparent Model for Ranking Jobs

> **Decision served:** the ranking engine a local-first desktop app should ship: how to score
> a posting on **fit** (can I get it?) and **quality** (do I want it?), transparently enough that
> the user can trust and tune it.
> **Method:** primary web research + academic retrieval/ranking literature; access date 2026-10-04.
> **Design constraint:** must run on-device, offline-capable, with an optional LLM for narration —
> never an LLM-only black box.

---

## 1. Why two axes, not one score

The upstream framework's `04-job-evaluation.md` blends five dimensions into one weighted number:

| Upstream dimension | Weight | Axis in this model |
|---|---:|---|
| Technical Skills Match | 30% | **Fit** |
| Experience Match | 25% | **Fit** |
| Behavioral/Culture Fit | 15% | **Quality** |
| Location & Logistics | pass/fail | **Gate** |
| Career Alignment & Motivation | 30% | **Quality** |

Blending fit and quality into one number is wrong for the user's decision: a role you'd easily get
at a bad company should not outrank a stretch role at a great one. **Separate the axes, show both,
and let the user weight them.** This is also a competitive differentiator — no incumbent exposes
*why* a role scored as it did.

```
Rank score = w_fit·Fit + w_quality·Quality      default w_fit = 0.65, w_quality = 0.35
```

Both axes are 0–100. Weights are user-adjustable and persisted; defaults lean "fit" because a
high-quality role you can't win has low expected value.

---

## 2. Stage 0 — Hard gates (VETO, never scored)

Run gates before any scoring; a failed gate excludes the role from the ranked list but records the
reason and lets the user override (the human is the tiebreaker, as upstream's rubric insists).

| Gate | Rule | Verdict |
|---|---|---|
| **Work eligibility** | Read citizenship/PR/clearance wording verbatim. Named requirement or security clearance → fail unless the candidate's declared status satisfies it. Silence = *unverified*, not permission. | VETO / FLAG |
| **Language** | Role's required working language(s) vs candidate's declared levels. Undeclared required language → fail. Bar plausibly above declared level → FLAG (score, warn). | VETO / FLAG |
| **Location / work mode** | Commute range or remote OK → pass. Mandatory relocation or travel beyond stated tolerance → veto unless overridden. | VETO / FLAG |
| **Compensation floor** | If Offer-level comp < user's hard minimum (or unknown + user demands known) → flag/veto per user preference. | VETO / FLAG |
| **Deadline** | Expired or closing-within-N-days → surfaced separately ("Closing soon"), never silently dropped. | FLAG |
| **Work authorization / right-to-work** | Permit class vs role requirement (e.g., "must have EU work permit"). | VETO / FLAG |

Every gate returns `PASS | FLAG | VETO` plus the **exact quoted evidence** and a confidence.
VETOes are reported to the user with the quote — the user may know something the profile doesn't.

---

## 3. Fit score — can I get it?

Fit answers: *how well does the candidate meet the stated requirements?* Hybrid lexical + semantic,
because pure keyword matching misses synonyms and pure embeddings miss hard constraints
(a known limitation of embedding-only retrieval [1]).

| # | Dimension | Weight | Signals & method |
|---|---|---:|---|
| F1 | **Skills match** | 0.35 | Extract required/preferred skills from the posting. Normalize against a skill lexicon (**ESCO**/**O*NET** synonyms: "JS"≈"JavaScript", "ML"≈"machine learning"). Compute: `skill_fit = α·Jaccard(must_have ∩ profile) + β·(nice_to_have coverage) + γ·KG_relatedness_bonus` (e.g. candidate has React → partial credit for Vue). Report exact matched skills and exact missing ones. |
| F2 | **Experience / seniority** | 0.25 | Extract role level (junior/mid/senior/staff) and years. Penalize `level_distance` (over- and under-qualification both hurt; over-qualification flagged as retention risk). Match **function**, not title — "Data Consultant" ≈ "Data Scientist" (upstream's rule). |
| F3 | **Domain / role similarity** | 0.15 | Cosine similarity between profile embedding and posting embedding (title + description). Use a small multilingual sentence encoder — `all-MiniLM-L6-v2` (384-dim, ~22M params) or a multilingual `e5-small`; upstream research used `all-MiniLM-L6-v2` and `multi-qa-MiniLM-L6-cos-v1` [1][2]. |
| F4 | **Location / work mode** | 0.10 | Tiered: remote-in-country pass; hybrid within commute = high; onsite within commute = high; relocation = low/FLAG; timezone overlap for remote-international. |
| F5 | **Education / certifications** | 0.10 | Required degree/cert as a soft factor (or gate if explicitly mandatory). Prefer equivalence reasoning over exact-title matching. |
| F6 | **Language nuance** | 0.05 | Beyond the gate: bonus when candidate exceeds the stated bar; small penalty when at/below an ambiguous bar. |

**Fit = Σ (Fᵢ · weightᵢ)**, normalized 0–100. Weights sum to 1.0.

**Explainability contract:** every dimension emits `{score, weight, matched[], missing[], evidence_spans[], confidence}`.
The UI shows a per-factor breakdown; an LLM may *narrate* it but may never change a number
(the "grounded explanation" pattern from JobMatchAI [3]).

---

## 4. Quality score — do I want it?

Quality answers: *how good is this offer, independent of whether I'll win it?*

| # | Dimension | Weight | Signals & method |
|---|---|---:|---|
| Q1 | **Compensation vs market** | 0.30 | Ratio of the offer's midpoint (or posted range) to a market benchmark for role×level×location. Sources, in order of preference: (a) user's own `salary_data.json`; (b) **Adzuna** salary endpoint or aggregator salary fields; (c) **BLS OES** (US, annual mean/median by occupation×area, CC0 public domain); (d) **levels.fyi** (paid API/MCP, granular total comp); (e) glassdoor-style self-reports. Score `= clamp(midpoint / p50, 0.5, 1.5)` mapped to 0–100. Show the benchmark basis. |
| Q2 | **Company signals** | 0.25 | Composite, each sub-signal 0–100: funding health & valuation trend (TrueUp/Caplight-style — flag down-rounds), **layoffs** (WARN notices, layoffs.fyi), **Glassdoor rating** + "recommend" %, **hiring velocity** / open-roles vs headcount, company age/size/tenure, review sentiment trend. Cap the weight when data is missing rather than imputing a neutral high. |
| Q3 | **Role & terms quality** | 0.15 | Remote quality (fully remote vs "hybrid" bait), benefits (equity, health, PTO, parental), contract type (permanent vs contractor), schedule flexibility, salary transparency, growth/promotion clarity, tech-stack modernity. |
| Q4 | **Posting legitimacy (anti-ghost)** | 0.20 | See §5. Penalizes likely ghost/stale postings so they don't waste applications. |
| Q5 | **Career alignment** | 0.10 | Does the role advance declared goals? Energizing tasks vs draining tasks (upstream's motivation filter); growth path; autonomy. Mostly LLM-assisted with evidence from the posting. |

**Quality = Σ (Qᵢ · weightᵢ)**, 0–100.

---

## 5. Ghost-job / red-flag detection (Q4 detail)

Ghost jobs are the specific failure mode that makes naive ranking harmful — it sends users to
apply where nobody is hiring. Detection is evidence-based and probabilistic; **never label a single
posting a "ghost job," report a risk rate** (the Unlisted methodology's own stance).

**Signals, ranked by reliability** (from the 2026 data):

| Signal | Evidence | Reliability |
|---|---|---|
| Posting age > 90 days | **27.6%** of open postings dated >90 days; 15.9% >180 days; median 35 days (**Unlisted Ghost Jobs Report, Sep 2026**, 619,116 postings across 15 ATS) | High |
| Reposted within 30 days | **3.1%** of closed postings reappear as new postings (same employer/role) | High |
| Stale by ATS | Ashby 34.4% >90d; Greenhouse 32.2%; SmartRecruiters 26.7%; Workday 16.2% (**Unlisted**) | High |
| Hiring freeze / layoffs in the news | Combine with Q2 | High |
| Vague/generic description, no specific program/team | Multiple 3P guides | Medium |
| No salary range / no hiring timeline | Medium |
| Below-market comp for senior responsibilities | Medium |
| "Pending funding", "evergreen", "talent pool", "subject to award" | Medium |
| Same role across many aggregators with differing company names | Medium |
| Sentiment: 40% of employers admit posting with no intent to fill (Resume Builder survey, cited by Sen. Gallego, Jun 2026); ~1-in-5 listings never filled | Context |

**Q4 score:** start at 100, subtract weighted penalties; expose each triggered signal with its
source. A posting with 2–3 strong signals is **deprioritized, not deleted** — and if it came from
an official ATS board (employer-posted), give a legitimacy bonus over aggregator-recycled listings.

**Also surfaced:** recruitment-fraud / scam patterns (pay-to-apply, off-platform messaging,
implausible pay for remote work) as a separate hard-red-flag badge.

---

## 6. The full formula (transparent & inspectable)

```python
def rank(posting, profile, prefs):
    gates = run_gates(posting, profile, prefs)      # PASS | FLAG | VETO + evidence
    if gates.has_veto and not prefs.allow_override:
        return Excluded(reason=gates.veto_reasons)

    fit = weighted_mean([
        (f1_skills(posting, profile),        0.35),
        (f2_experience(posting, profile),    0.25),
        (f3_semantic(posting, profile),      0.15),   # on-device embedding cosine
        (f4_location(posting, profile),      0.10),
        (f5_education(posting, profile),     0.10),
        (f6_language(posting, profile),      0.05),
    ])
    quality = weighted_mean([
        (q1_comp(posting, market_data),      0.30),
        (q2_company(posting, company_data),  0.25),
        (q3_terms(posting),                  0.15),
        (q4_legitimacy(posting),             0.20),
        (q5_alignment(posting, profile),     0.10),
    ])

    score = prefs.w_fit * fit + prefs.w_quality * quality   # default 0.65 / 0.35
    return Ranked(
        score=score, fit=fit, quality=quality,
        factors=[F1..F6, Q1..Q5],           # each with evidence + confidence
        gates=gates,
        confidence=coverage_of_available_data(),
        verdict=threshold(score),           # see below
    )
```

**Verdict thresholds** (mapping, not decision): Strong ≥75 · Good 60–74 · Moderate 45–59 ·
Weak 30–44 · Poor <30. Separate **"Closing soon"** and **"Below threshold / Excluded"** sections,
exactly as upstream's `/rank` report does.

**Confidence.** Every factor carries a confidence equal to its data coverage. Missing salary data,
missing company signals, or an unparseable posting lowers confidence and is shown explicitly —
never silently imputed to neutral. A high score at low confidence is presented as a *lead*, not a
verdict.

---

## 7. Data sources & offline strategy

| Tier | Available offline (default) | Requires network (opt-in) |
|---|---|---|
| Posting content | User-pasted / ATS-board JSON already stored | Live fetch of the posting URL |
| Skills taxonomy | Bundled **ESCO** (multilingual, EU) or **O*NET** subset | — |
| Embeddings | Bundled small encoder (`all-MiniLM-L6-v2` / multilingual `e5-small`), CPU inference | — |
| Salary benchmark | User's `salary_data.json` | **Adzuna**, **BLS OES** (bulk download), **levels.fyi** (paid) |
| Company signals | User-cached research (30-day TTL) | WARN/layoffs.fyi, Glassdoor, TrueUp/Caplight |
| LLM narration / drafting | **Local Ollama** models | BYO API key (Anthropic/OpenAI/Gemini/OpenRouter) |

Design rule: the **core score is deterministic and runs offline**; network enrichments improve
confidence but never gate the ranking. This matches the product's local-first promise.

---

## 8. Calibration loop (make it get better, transparently)

Upstream's best idea is recalibrating the rubric from real outcomes (`/outcome` → `/setup`). Keep it,
but make it explicit:

1. Track outcome per application: `drafted → applied → interview → offer → hired / rejected / no_response`.
2. Once N outcomes accumulate, compute **per-factor correlation with interview/offer** (simple,
   inspectable statistics — e.g. logistic regression on the factor vector with the user's own data).
3. Propose **weight adjustments** to the user with rationale ("your interviews correlate most with
   F1 skills and Q1 comp; F5 education never discriminates — lower its weight?"). User accepts or not.
4. Never auto-change weights silently; always show what changed and why. Report sample size and
   uncertainty; suppress suggestions when N is too small.

This is a genuine differentiator: incumbents' matching is a static black box; here the ranking is
explainable *and* personalized to what actually worked for this user.

---

## 9. Worked example (illustrative)

Candidate: mid-level data engineer, Python/SQL/dbt/Airflow, Spanish native / English B2, Madrid,
remote-preferred, salary floor €45k.

Posting: "Senior Data Engineer", dbt/Airflow/Snowflake, remote-Spain, €55–70k, posted 12 days ago,
Greenhouse board, company with a recent down-round and 3 Glassdoor layoff mentions.

| Factor | Score | Notes |
|---|---:|---|
| Gates | FLAG | Title says "Senior" vs declared mid — not a hard fail, warn |
| F1 Skills | 88 | dbt/Airflow/Python all matched; Snowflake missing |
| F2 Experience | 58 | Seniority gap |
| F3 Semantic | 82 | Same domain |
| F4 Location | 95 | Remote in-country |
| F5 Education | 100 | No degree requirement |
| F6 Language | 100 | English B2 clears "professional" |
| **Fit** | **81.3** | |
| Q1 Comp | 92 | Midpoint €62.5k vs Madrid p50 ~€52k |
| Q2 Company | 35 | Down-round + repeated layoff signals |
| Q3 Terms | 78 | Equity, permanent, remote |
| Q4 Legitimacy | 85 | Fresh, employer-posted, salary shown |
| Q5 Alignment | 80 | Modern stack, growth path |
| **Quality** | **63.9** | |
| **Rank score** | **75.2** | Strong overall, dragged by company risk |

The UI shows exactly this: fit is excellent, company health is the weakness — the user decides.

> **Illustrative, not reproducible (S6.3, 2026-10-17).** The factor column above does not
> average to the axes printed beside it: weighted by §6's own weights it gives Fit **82.1**
> (not 81.3) and Quality **73.05** (not 63.9). The Rank figure of 75.2 follows from the axes
> *as printed* (`0.65·81.3 + 0.35·63.9`), which is what S6.3's golden test pins; the same
> test also pins what the shipped model computes from this example's inputs (Fit 67.44,
> Quality 66.56, Rank 67.14), so the two are never confused. Treat this table as an
> illustration of the *shape* of a ScoreCard, not as expected output.

---

## Sources

1. Synapse / resume–job matching pipeline — Sentence-BERT `all-MiniLM-L6-v2` + FAISS + semantic reranking, ~120k postings: <https://arxiv.org/pdf/2604.02539> (2026).
2. AI-driven resume analysis — `multi-qa-MiniLM-L6-cos-v1` + `all-MiniLM-L6-v2`, semantic + keyword hybrid: <https://aclanthology.org/2025.clicit-1.51.pdf> (2025).
3. JobMatchAI — hybrid retrieval (BM25 + ANN + skill KG) and **white-box multi-factor utility** with user-adjustable weights (skill 0.35, experience 0.25, location 0.15, salary 0.10, semantic 0.10, company 0.05): <https://aclanthology.org/2026.acl-demo.52.pdf> (2026).
4. ConFit / ConFit v2 — contrastive resume–job embeddings, hard-negative mining: <https://arxiv.org/html/2401.16349v1>, <https://arxiv.org/html/2502.12361v3>.
5. Resume–job compatibility with GNNs; text-embedding-3-large + phrase definitions, R²=0.803, 25% over similarity baseline: <https://dl.acm.org/doi/10.1145/3787330.3787359> (2025).
6. Unlisted, **Ghost Jobs Report, September 2026** (619,116 postings, 15 ATS): <https://unlisted.careers/ghost-jobs/report/2026-09> (2026-09-30).
7. Ghost-job prevalence & policy — Resume Builder survey (40% of employers; ~1-in-5 never filled), Sen. Gallego letter, NY Senate bill: <https://www.gallego.senate.gov/news/press-releases/gallego-demands-answers-from-trump-administration-on-combatting-ghost-jobs> (2026-06-18); <https://spectrumlocalnews.com/nys/central-ny/politics/2026/04/30/...> (2026-04-30).
8. Salary benchmarks — Lotus/Levels.fyi pay reports & API: <https://www.levels.fyi/2025/report.md>; BLS Occupational Employment & Wage Statistics (OES, public domain): <https://download.bls.gov/pub/time.series/oe/oe.datatype>.
9. Company signals — Glassdoor Worklife Trends 2026 ("forever layoff", senior-leadership disconnect): <https://www.glassdoor.com/blog/worklife-trends-2026>; TrueUp company trajectory/layoff/valuation data: <https://www.trueup.io/co/papaya-global>.
10. Skill taxonomies — **ESCO** (European Skills, Competences, Occupations), **O*NET** (US DOL) — bundled offline dictionaries.

*Upstream rubric cross-reference:* `/tmp/boba-upstream/.claude/skills/job-application-assistant/04-job-evaluation.md` (gates, weights, thresholds) — see `upstream-analysis.md`.
