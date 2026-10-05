# Integrations Landscape — Job Sources, Email, and Legal Risk

> **Decision served:** which job-board and email integrations to build for a local-first,
> cross-platform desktop job-search app, and in what tier/order.
> **Method:** parallel web research, primary sources preferred; endpoint probes performed
> 2026-10-04 where noted ✅. This is research, **not legal advice**.
> **Access date for all sources:** 2026-10-04 unless stated.

Access legend for job sources:
- **Official API** — vendor-documented, sanctioned.
- **Public undocumented** — returns public JSON but no official developer product; may break.
- **Scraping only** — no machine endpoint; parse HTML/internal API. **High legal risk.**
- **Auto-apply** flags ability to *submit* an application, not merely read.

---

## 1. Global aggregators / boards (Group A)

| Source | Access | Auth | Cost | Rate limits | Auto-apply | Coverage |
|---|---|---|---|---|---|---|
| **LinkedIn** | Official API **partner-only** (Talent/Apply Connect); no candidate search API; `jobs-guest` scraping is the only public path | OAuth 2.0 + partner contract | No public price; contract | Undisclosed | ❌ No (partner ATS only) | Global |
| **Indeed** | Publisher/Job-Search API **retired** (closed 2020–2022, dark 2024); remaining APIs employer-side | OAuth (approved partner) | Free APIs; sponsored spend | e.g. `findEmployerJobsPartner` 5 rps | ❌ No (Indeed Apply is inbound) | Global (~60+) |
| **Glassdoor** | **No API** — v1 partner endpoint returns **HTTP 410 Gone** (rechecked 2026-09-30); scraping behind Cloudflare + login | n/a (partner program closed) | n/a | n/a | ❌ No | Global, US-heavy |
| **ZipRecruiter** | Publisher **ZipSearch deprecated 2025-03-31**; employer/ATS Partner Toolkit remains | API key (Basic), email onboarding | Free organic; paid sponsored | Not published | ❌ No (Apply Webhook = inbound) | US/UK |
| **Google Jobs (direct)** | **Scraping only**; ToS forbids automated queries | n/a | n/a | n/a | ❌ No | Global |
| **Google Jobs via SerpApi** | Third-party SaaS scraping Google | `api_key` | $0 (250/mo) → $25 (1k) → $75 (5k)… | throughput/hr per plan (50→200k+) | ❌ No | Global |
| **Google Cloud Talent Solution** | Official GCP API, **indexes only jobs you ingest** | GCP service acct/OAuth | Free ≤10k searches/mo, then $0.0015; jobs $0.00025 ea. | per-project quotas | ❌ No | Global (your data) |
| **Adzuna API** | Official REST | `app_id`+`app_key` | Free; raises on request | Official ToS: 25/min, 250/day, 1k/week, 2.5k/mo | ❌ No | 20+ countries |
| **JSearch (RapidAPI)** | Third-party official API (Google-for-Jobs aggregate) | RapidAPI key | Free 200/mo; $25/10k; $75/50k; $150/200k | 1000/hr free; 5/10/20 rps paid | ❌ No | Global |
| **Jooble API** | Official REST `POST /api/{key}`; per-domain key | API key in URL path | Free **500 requests/key lifetime**; paid undisclosed | Not published | ❌ No | 60+ countries |
| **Arbeitnow API** | Official free public JSON, **no auth** | None | Free | Not stated | ❌ No | Europe (DE-centric) |
| **RemoteOK API** | Official free public JSON `/api`, no auth | None | Free (dofollow backlink required) | Newest 100 only; no pagination; `/remote-jobs.rss` = **410** ✅ | ❌ No | Global remote |
| **WeWorkRemotely** | Official API + RSS, no auth | None | Free | Not stated | ❌ **Explicitly disallowed** — apply must route via site | Global remote |
| **Remotive API** | Official free public JSON, no auth | None | Free (attribution) | asks ≤4 fetches/day; blocks >2 req/min | ❌ No | Global remote |
| **Himalayas API** | Official free public JSON + OpenAPI 3.1, no auth | None | Free (attribution) | 429 when exceeded; cached 24h; ≤20/page | ❌ No | Global remote |
| **Wellfound (AngelList)** | **No public API**; recruiter-side OAuth MCP only; internal GraphQL gated (DataDome) | OAuth 2.1 (recruiter) | n/a | n/a | ❌ No | Global (startups) |
| **HN "Who is Hiring"** | Algolia HN Search API + Firebase, no key | None | Free | ~10k req/hr soft | ❌ No (apply links only) | Global |

**Notes**
- **LinkedIn** has no developer path to listings; User Agreement §8.2 bans scraping/bots and
  automated messaging. Automation = account-ban risk. *Least-contested pattern:* user-driven
  autofill on the user's own session, with the human clicking submit.
- **Google-for-Jobs resellers (SerpApi, JSearch)** work but rest on scraping Google in violation
  of Google's ToS; **SerpApi is under active litigation (Google v. SearchGuard/SerpApi)**, so
  treat as legal-risk with abrupt-interruption risk.
- **Microsoft Learn says LinkedIn Job Posting API is "not accepting new partnerships"**; Apply
  Connect/Easy Apply deliver applicants *to* an ATS, not outward.
- **Adzuna:** secondary claims of a 5,000/mo free tier are **unverified** against the official ToS
  (official limits are lower).
- **RemoteOK/Remotive/Himalayas/WWR** require attribution/link-backs.

*Sources: `learn.microsoft.com/linkedin/talent/...`, `linkedin.com/legal/user-agreement`,
`docs.indeed.com`, `jobspipe.dev`, `ziprecruiter.com/partner/documentation`,
`policies.google.com/terms`, `serpapi.com/pricing`, `cloud.google.com/talent-solution/pricing`,
`developer.adzuna.com/docs/terms_of_service`, `help.jooble.org/.../rest-api-documentation`,
`arbeitnow.com`, `remoteok.com/api`, `weworkremotely.com`, `remotive.com/api`, `himalayas.app/jobs/api`,
`hn.algolia.com/api`.*

---

## 2. ATS public job-board JSON APIs (Group B) — the cleanest programmatic source

| ATS | Public read endpoint | Auth (read) | Apply programmatically |
|---|---|---|---|
| **Greenhouse** | ✅ `GET https://boards-api.greenhouse.io/v1/boards/{token}/jobs?content=true` | None | ⚠️ `POST …/jobs/{id}` needs **employer's board API key** |
| **Lever** | ✅ `GET https://api.lever.co/v0/postings/{company}?mode=json` (EU: `api.eu.lever.co`) | None | ⚠️ `POST …?key=APIKEY` needs **employer Super-Admin posting-form key**; rate-limited |
| **Ashby** | ✅ `GET https://api.ashbyhq.com/posting-api/job-board/{name}?includeCompensation=true` | None | ❌ hosted `applyUrl` only |
| **Workable** | ✅ `GET https://apply.workable.com/api/v1/widget/accounts/{slug}?details=true` | None | ❌ `application_url` only |
| **SmartRecruiters** | ✅ `GET https://api.smartrecruiters.com/v1/companies/{id}/postings` | None for public read | ❌ no documented public apply endpoint |

All five read endpoints probed live and returned **200** ✅ (2026-10-04). **Applying programmatically
is employer-gated everywhere** — there is no candidate-side sanctioned auto-apply path in any ATS.
Workable's widget API is **public-but-undocumented** (may change); SmartRecruiters' public GET is
widely used but not contractually guaranteed.

*Sources: `developers.greenhouse.io/job-board.html`, `github.com/lever/postings-api`,
`developers.ashbyhq.com/docs/public-job-posting-api`, `apply.workable.com/api/v1/widget/...`,
`developers.smartrecruiters.com/docs/posting-api`.*

---

## 3. Regional / non-English (Group C)

| Source | Access | Auth | Auto-apply | Coverage |
|---|---|---|---|---|
| **Computrabajo** | **Scraping only** (no public API); 3rd-party MCP can apply using **user session cookie** | n/a | ⚠️ 3rd-party/cookie | 19 LatAm countries + ES |
| **OCC Mundial** | **Scraping only**; `/empleos` returned **403** to bots ✅ | n/a | ❌ | Mexico |
| **Bumeran** | **Scraping only** — internal Navent `POST /api/avisos/searchV2`, Cloudflare; 100/page cap | n/a | ❌ | AR, MX, PE, EC, VE (+CL Laborum, PA Konzerta) |
| **InfoJobs** | **Official API** `developer.infojobs.net` | OAuth 2.0 (app+user) | ✅ **Yes** — `POST /offer/{id}/application` for the authenticated candidate | Spain |
| **StepStone** | Employer **XML feed only**; no read API | per-contract creds | ❌ (posting only) | DACH: DE/AT/BE/NL |
| **Xing** | Employer E-Recruiting API + "Apply with XING" | OAuth 2.0 + contract | ⚠️ employer/ATS side | DACH: DE/AT/CH |
| **Otta / Welcome to the Jungle** | **No public candidate API**; WTTJ Solutions API token/scope-gated | Bearer/OAuth | ❌ | UK/EU (FR-heavy) |

**The standout: InfoJobs** — the only source surveyed with a **sanctioned, candidate-authorized
auto-apply path** (OAuth, search + submit with killer/open questions).

*Sources: `developer.infojobs.net/documentation/operation-list/index.xhtml`, `apify.com` actors,
`github.com/kalil0321/ats-scrapers/pull/123`, `api.stepstone.com/feed-types/`,
`dev.xing.com/partners/job_integration/api_docs`, `developers.welcomekit.co/jobs-api/jobs/list-jobs`.*

---

## 4. Auto-apply risk summary

> **⚠ Product decision 2026-10-04 (deviation from this research — planning audit P1-12):** LinkedIn and Indeed session-board auto-apply **ships in MVP**, opt-in per Source, executed only by the core with caps, pacing, Outbox hold, Kill Switch and Audit Log (PRD FR-30, FR-91; architecture §17 Q5, ADR-005). This overrides the earlier "do not build" / "out of bounds" recommendation below. The residual account-ban and ToS risk is accepted by the user, shown at opt-in, and re-confirmed every 30 days. LATAM session boards (Computrabajo, OCC, Bumeran) follow in v1.1. The risk facts below remain accurate and are kept as-is. *Note added by pm (analyst offline).*

| Risk level | Sources |
|---|---|
| **Sanctioned candidate auto-apply** | InfoJobs (OAuth, candidate-authorized) |
| **Employer-gated apply endpoint (not candidate-usable)** | Greenhouse, Lever (need employer key); ZipRecruiter/Indeed Apply (inbound); Xing/StepStone (posting feeds) |
| **No apply path** | Ashby, Workable, SmartRecruiters, OCC, Bumeran, Otta/WTTJ, RemoteOK, Remotive, Himalayas, Arbeitnow, HN, Adzuna, JSearch, SerpApi/Google |
| **Explicitly prohibited** | **LinkedIn** (UA §8.2), **WeWorkRemotely**, **Google** (automated queries) |
| **High legal risk / grey** | Computrabajo (cookie), Glassdoor, Indeed, ZipRecruiter, Wellfound, StepStone, Bumeran, OCC |

**Cross-cutting:** the read layer is closing and the write layer is gated. Durable free reads =
niche remote boards + **ATS public boards**. Only InfoJobs offers legitimate candidate auto-apply.

---

## 5. Email integrations

### 5.1 Comparison

| | Gmail API | Microsoft Graph (Outlook/M365) | IMAP/SMTP OAuth2 | IMAP/SMTP app-password |
|---|---|---|---|---|
| Auth model | OAuth desktop **loopback** + PKCE; restricted scopes for reading | OAuth **delegated** + PKCE/loopback; **no admin consent** for user `Mail.*` | XOAUTH2 | Static app password |
| Send | `gmail.send` (**Sensitive**, no CASA) | `Mail.Send` (delegated; personal accounts OK) | provider SMTP | provider SMTP |
| Read | `gmail.readonly/modify` (**Restricted → CASA**) | `Mail.Read`/`Mail.ReadWrite` (delegated) | `https://mail.google.com/` = restricted | n/a |
| Cost | Free (possible usage charges later 2026) | Free | Free | Free |
| Rate limits | 1.2M units/min/project; 6,000 units/min/user; per-user send caps not raisable | **10,000 req/10 min per app+mailbox; 4 concurrent; 150 MB/5 min** | provider caps | provider caps |
| Desktop suitability | Loopback OK; CASA ambiguity for reading | **Best** for M365/Outlook | Good w/ XOAUTH2 | Simplest, fragile, dying |
| ToS difficulty | High if reading | Low–medium | Medium | Low |

### 5.2 Key findings
- **Gmail:** send-only avoids CASA; **reading recruiter replies requires a restricted scope →
  annual CASA security assessment** unless the app truly never stores/transmits data through a
  server. Google's own pages **conflict** on whether pure on-device processing is exempt — treat
  as *plausible but unconfirmed*; get written confirmation from the OAuth review team before
  betting the architecture. Desktop **loopback OAuth is still supported** for the "Desktop app"
  client type (OOB copy/paste is dead; RFC 8252 loopback is the recommended native flow).
  Verification: brand 2–3 days, restricted-scope review potentially weeks. CASA third-party
  assessor fees ~$540–$8,000 (vendor-listed, not official).
- **Microsoft Graph:** delegated `Mail.Read`/`Mail.Send` need **no admin consent**, work for
  personal accounts via `/common`, and support PKCE loopback — the best desktop story. **Do not
  use app-only permissions** (admin consent + all-mailbox access). 202 = accepted, not delivered
  (don't retry).
- **IMAP/SMTP:** Gmail basic auth disabled (app passwords only, require 2SV); Exchange Online
  basic auth and app passwords disabled. Generic providers still allow app passwords, but the
  direction of travel is OAuth2/XOAUTH2.

### 5.3 Privacy / architecture implications
- A **truly local app** (no vendor backend, no telemetry carrying mail data) is the strongest
  position: it shrinks Gmail verification/CASA scope and keeps the vendor out of GDPR
  controller/processor duties. **Avoid becoming a backend by accident** (cloud sync, crash dumps,
  hosted OAuth proxy, server-side parsing).
- Tokens must be encrypted at rest — use the OS keychain (macOS Keychain / Windows DPAPI /
  libsecret), never a plaintext config file.

*Sources: `developers.google.com/workspace/gmail/api/auth/scopes`,
`developers.google.com/workspace/gmail/api/reference/quota`,
`developers.google.com/identity/protocols/oauth2/native-app`,
`developers.google.com/workspace/workspace-api-user-data-developer-policy`,
`learn.microsoft.com/graph/api/user-sendmail`, `learn.microsoft.com/graph/throttling-limits`,
`learn.microsoft.com/exchange/.../deprecation-of-basic-authentication-exchange-online`,
`rfc-editor.org/info/rfc8252`.*

---

## 6. Legal / ToS landscape (read before shipping any automation)

> **⚠ Product decision 2026-10-04 (deviation from this research — planning audit P1-12):** LinkedIn and Indeed session-board auto-apply **ships in MVP**, opt-in per Source, executed only by the core with caps, pacing, Outbox hold, Kill Switch and Audit Log (PRD FR-30, FR-91; architecture §17 Q5, ADR-005). This overrides the earlier "do not build" / "out of bounds" recommendation below. The residual account-ban and ToS risk is accepted by the user, shown at opt-in, and re-confirmed every 30 days. LATAM session boards (Computrabajo, OCC, Bumeran) follow in v1.1. The risk facts below remain accurate and are kept as-is. *Note added by pm (analyst offline).*

| Area | Position |
|---|---|
| **LinkedIn** | UA §8.2 bans scraping and bots; dedicated Crawling T&C; enforcement history (hiQ, Proxycurl/Nubela 2025, **ProAPIs et al. N.D. Cal. 3:25-cv-08393, Oct 2025**). Practical risk: **account bans**, not generally CFAA, for logged-in automation. |
| **Indeed** | ToS §5 bans automation of Indeed Apply outside official vendors; caps daily applications. |
| **Glassdoor** | ToU bans automated agents and scraping without written permission. |
| **ZipRecruiter** | robots.txt deny-by-default, blocks many AI/LLM bots; ToS reportedly bans automated collection. |
| **CFAA** | **Van Buren (2021)**: "exceeds authorized access" = access-with-authorization then reaching off-limits areas; motive alone insufficient. **hiQ v. LinkedIn (9th Cir. 2022)**: scraping *public* data likely not CFAA "without authorization." But **ToS breach/contract, trespass to chattels, DMCA §1201** remain live — and hiQ ultimately *lost* on the User-Agreement claim. Fake accounts / access-control circumvention materially raise risk. |
| **GDPR (EU/UK/ES)** | A user's **purely personal/household** job search is likely exempt (Recital 18). **But the vendor "providing the means for processing" stays in scope** — and any server-side component (sync, telemetry, hosted OAuth) makes you a controller/processor. Parsing recruiter email processes **third-party personal data**; minimisation, retention, and (if profiling/auto-decisions) Art 22 concerns apply. ICO recruitment guidance and EDPB web-scraping guidelines (2025) are the reference points; Spain supplements GDPR with LOPDGDD 3/2018. |

**Practical safe zone:** user-driven autofill on the user's own authenticated session, human
clicks submit; native one-click apply routes (LinkedIn Easy Apply, Indeed Apply); reading public
ATS boards; InfoJobs' sanctioned candidate API. **Out of bounds for a shipping product:** silent
auto-submit on LinkedIn/Indeed/Glassdoor/ZipRecruiter, bulk scraping with fake accounts, or
circumventing access controls.

*Sources: `linkedin.com/legal/user-agreement`, `linkedin.com/legal/crawling-terms`,
`indeed.com/legal`, `glassdoor.com/about/terms`, `ziprecruiter.com/robots.txt`,
supremecourt.gov *Van Buren* 19-783, cdn.ca9.uscourts.gov *hiQ* 17-16783,
`eur-lex.europa.eu` GDPR Recital 18 & Art 2(2)(c), `ico.org.uk` recruitment guidance,
`edpb.europa.eu` web-scraping guidelines, `cnil.fr` legitimate-interest sheet.*

---

## 7. Recommended tiered integration strategy

> **⚠ Product decision 2026-10-04 (deviation from this research — planning audit P1-12):** LinkedIn and Indeed session-board auto-apply **ships in MVP**, opt-in per Source, executed only by the core with caps, pacing, Outbox hold, Kill Switch and Audit Log (PRD FR-30, FR-91; architecture §17 Q5, ADR-005). This overrides the earlier "do not build" / "out of bounds" recommendation below. The residual account-ban and ToS risk is accepted by the user, shown at opt-in, and re-confirmed every 30 days. LATAM session boards (Computrabajo, OCC, Bumeran) follow in v1.1. The risk facts below remain accurate and are kept as-is. *Note added by pm (analyst offline).*

### Tier 0 — MVP (safe, free, no auth, high signal)
| Integration | Why |
|---|---|
| **ATS public boards** (Greenhouse, Lever, Ashby, Workable, SmartRecruiters) | Official/public JSON, unauthenticated, employer-posted (low ghost rate), structured salary on Ashby |
| **Remote/tech free feeds** (Arbeitnow, RemoteOK, Remotive, Himalayas, WeWorkRemotely-RSS, HN Who's Hiring via Algolia) | Official/public, no auth, attribution-friendly |
| **Adzuna API** + **Jooble API** | Official aggregator APIs, global/regional coverage |
| **Manual URL/text paste** for anything else (LinkedIn/Indeed/Glassdoor/etc.) | Legally clean; the user supplies the posting, as upstream's `/apply` already supports |
| **Email: send-only via user's own SMTP/Graph/Gmail** (consent-gated, draft-only), **IMAP read optional and local** | Avoids CASA for MVP; sends follow-ups/thank-yous the user approves |
| **Email: manual/`.eml`/clipboard import for recruiter replies** | No OAuth-scope burden; classify locally |

### Tier 1 — Post-MVP (sanctioned auth or explicit partnership)
| Integration | Requirement |
|---|---|
| **InfoJobs** candidate OAuth (search **and** sanctioned apply) | Official API; Spain launch leverage |
| **Microsoft Graph** full read/send | Delegated user consent, loopback; no admin consent |
| **Gmail restricted-scope read** (recruiter-reply parsing) | Budget for verification + CASA (or prove on-device exemption in writing first) |
| **SerpApi/JSearch Google-Jobs** | Accept supplier legal risk; make it user-configurable, not default |
| **Computrabajo / OCC / Bumeran** | Only with a user-session-cookie model and clear ToS warnings; confidential-risk |

### Tier 2 — Later / partner-only
Indeed, LinkedIn (partner/Apply Connect), Glassdoor, ZipRecruiter, StepStone, Xing, Otta/WTTJ,
Wellfound — all require closed partner agreements, are employer-side, or are prohibited.

### Risk matrix

| Integration | Legal risk | Product risk (breakage) | Cost | Build effort | Recommendation |
|---|---|---|---|---|---|
| ATS boards (public JSON) | **Low** | Low–med | Free | Low | **MVP** |
| Remote feeds (Arbeitnow/RemoteOK/Remotive/Himalayas/WWR/HN) | **Low** | Med (hosted, no SLA) | Free | Low | **MVP** |
| Adzuna / Jooble | Low | Low | Free/low | Low | **MVP** |
| Manual paste / URL fetch | **Lowest** | Low | Free | Trivial | **MVP** |
| Email send-only (SMTP/Graph) | Low | Low | Free | Med | **MVP** |
| Email read (IMAP local) | Low–med (3rd-party data) | Med | Free | Med | **MVP-optional** |
| Gmail restricted read | **Med–high (CASA)** | Med | Assessor $$ | High | **Tier 1** |
| InfoJobs OAuth (apply) | **Low (sanctioned)** | Low | Free | Med | **Tier 1** |
| SerpApi/JSearch | **High (Google ToS/litigation)** | High | $25–150/mo | Low | **Tier 1, opt-in** |
| Computrabajo/OCC/Bumeran scraping | **High** | High | Apify $$ | Med–high | **Tier 1, warned, opt-in** |
| LinkedIn automation | **Prohibited** | Ban risk | — | — | ~~Do not build~~ → **MVP, per-Source opt-in, user-accepted risk** (product decision 2026-10-04) |
| Indeed/Glassdoor/ZipRecruiter automation | **Prohibited** | Ban risk | — | — | Indeed: ~~Do not build~~ → **MVP, per-Source opt-in, user-accepted risk** (2026-10-04); Glassdoor/ZipRecruiter: do not build |

**Bottom line:** ship a **read layer from ATS boards + free remote/Aggregator APIs + manual paste**,
a **consent-gated email layer that defaults to send-only**, and reserve every authenticated or
scraping integration for opt-in Tier 1 with explicit ToS warnings. The product's defensible
position is *aggregation + scoring + tailoring on the user's machine*, not auto-submission.
