# Competitive Teardown — AI Job-Search Tools

> **Decision served:** where a local-first, cross-platform **desktop** app can win.
> **Method:** web research; primary/official pricing pages preferred. Access date 2026-10-04
> unless noted. Pricing is USD and US-default; regional/app-store/A-B pricing varies.
> **[UNVERIFIED]** marks claims not confirmed against a first-party page.

---

## 1. Per-tool profiles

| Tool | Positioning | Key features | Pricing (free → paid) | Platform | Data posture |
|---|---|---|---|---|---|
| **Teal** (TealHQ) | Freemium job-search **organizer**: unlimited resume builder + CRM tracker; **no auto-apply** | Unlimited resumes/tracking, AI bullets/summaries/cover letters, Match Score, Analysis Mode, Chrome ext. | Free Forever $0; **Teal+ $13/7d (~$56/mo eq.), $29/30d, $79/90d**; no annual. Free = top-5 keywords only, limited AI credits | Web + Chrome ext. | Cloud SaaS |
| **Huntr** | All-in-one CRM + AI tailoring with **free unlimited autofill** | Tracker (100 free jobs), AI tailored resumes, cover letters, resume checker, autofill, contacts/interview trackers | **Free** (2 tailored resumes, 100 jobs, unlimited autofill); **Pro $40/mo, $90/3mo, $160/6mo**; no annual | Web + ext. + mobile | Cloud SaaS |
| **Simplify** | Free **Copilot autofill** (100k+ sites) + tracker; paid adds AI | Autofill (Workday/Greenhouse/Lever…), tracker, AI resume/CL, custom Q&A, matching; "Autopilot" auto-submit beta **[UNVERIFIED]** | **Free forever**; **Simplify+ $19.99/wk, $39.99/mo, $89.99/3mo**; no trial/annual/refund | Chrome/Safari/Firefox(beta) ext. + web | Cloud; privacy policy flagged last updated **2021** (3P) |
| **Jobscan** | **ATS keyword-match optimizer** | Match Rate, ATS/recruiter findings, skill gaps, formatting checks, AI Optimize, tracker | Free **5 scans/mo**; **Premium $49.95/mo or $89.95/qtr (~$29.98/mo)**; trial only on quarterly | Web + Chrome ext. | Cloud SaaS |
| **LazyApply** | **Mass auto-apply bot** | Automated applications (Indeed/Greenhouse/Dice/ZipRecruiter), referral emails, analytics, autofill, LinkedIn tools | **Basic $99/yr (15 apps/day); Premium $149/yr (150/day); Ultimate $999/yr (1,500/day)**; annual only; **requires a Gmail address** | Web + Chrome ext. | Cloud; Gmail-tied; ToS/reputation risk |
| **Sonara** | Hands-off **auto-apply autopilot** (relaunched under BOLD) | Continuous scanning, daily matches, auto-apply, tracking; light tailoring | **Conflicting**: Pulse $19.99/mo, Accelerate $49.99/mo, tier $79.99/mo vs 2026 3P ~$23.95/4wk (~$311/yr) or $71.40/yr. **[UNVERIFIED — site failed to load 2026-10-04]** 3P: 25–40% failure rates | Web | Cloud; price hidden until signup; trial auto-renews |
| **AIApply** | End-to-end co-pilot: resume/CL AI + **credit auto-apply** + Interview Buddy | Resume/CL builder, Auto Apply, 20M+ board, ATS scanner, translator, mock/live interview | Free (limited); **Premium $29/mo or $144/yr**; **Interview Buddy $19/mo**; **Auto Apply $49/mo (100 apps) … $199/qtr (750)**; 1 credit = 1 app | Web + ext. | Cloud (GCP); encryption, no selling |
| **Careerflow** | Budget **LinkedIn + resume AI toolkit** | Tracker (unlimited free), LinkedIn optimizer, resume/CL AI, ATS analyzer, skill-gap, autofill, mock interview (higher tier) | **Free**; **Premium $8.99/wk, $23.99/mo, $172.99/yr**; **Plus $19.99/wk, $44.99/mo, $299.99/yr** | Web + ext. | Cloud SaaS |
| **Kickresume** | Polished **resume/CL/website builder** + ATS check (no apply) | 40+ resume / 40+ CL templates, GPT-4.1 writer, ATS checker, Career Map, websites, mobile apps, job matcher | **Free** (4 templates, limited customization); **Premium $19/mo, $9/mo qtr, $4.50/mo yearly**; free for students/teachers | Web + iOS + Android | **EU-hosted, GDPR**; states no sharing, pentests |
| **Rezi** | **ATS-first resume builder**; no auto-apply | Rezi Score (23 metrics), AI keyword targeting, writer/editor, AI interview, Chrome ext., **Rezi Resume MCP** | **Free** (1 resume, 3 PDFs); **Pro $29/mo**; **Lifetime $149**; **Enterprise $99/mo per 200 users** | Web + ext. (+MCP) | **SOC 2**; cloud |
| **Jobright** | AI **matching + insider-referral agent** + one-click autofill | Profile-ranked matching (8M+ listings), 6-sec tailored CV, autofill, "Insider Connection" emails, live coach, AI auto-apply (beta) | **No public pricing page**; free daily credits. Turbo **[UNVERIFIED ~$17.99/wk, $39.99/mo, $89.99/qtr]**; no trial, no-refund reported | Web + ext. | Cloud; opaque pricing |

---

## 2. Feature matrix

| Tool | Tracker | Auto-submit | Resume tailoring | Cover letter | Interview prep | Chrome ext. | AI matching | Free tier | Entry paid (mo-eq.) |
|---|:--:|:--:|:--:|:--:|:--:|:--:|:--:|---|---|
| Teal | ✅ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | Generous | $29/30d |
| Huntr | ✅ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | Good | $26.66–40 |
| Simplify | ✅ | ⚠️ beta | ✅ | ✅ | ❌ | ✅ | ✅ | Strong | $39.99 |
| Jobscan | ✅ | ❌ | ✅ (keyword) | ❌ | ❌ | ✅ | ❌ | 5 scans/mo | ~$29.98 |
| LazyApply | ✅ | ✅ mass | ⚠️ minimal | ✅ | ❌ | ✅ | ✅ | ❌ | $8.25 ($99/yr) |
| Sonara | ✅ | ✅ hands-off | ⚠️ light | ⚠️ | ❌ | ❌ | ✅ | Trial only | $19.99–23.95* |
| AIApply | ✅ | ✅ credits | ✅ | ✅ | ✅ | ✅ | ✅ | Weak | $29 (+$49 auto) |
| Careerflow | ✅ | ⚠️ autofill | ✅ | ✅ | ✅ (Plus) | ✅ | ✅ | Free tools | $14.41–23.99 |
| Kickresume | ✅ | ❌ | ✅ | ✅ | ⚠️ | ❌ | ✅ | Strong | $4.50 |
| Rezi | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ❌ | 1 resume | $29 (or $149 life) |
| Jobright | ✅ | ⚠️ beta | ✅ | ⚠️ | ⚠️ | ✅ | ✅ | Credits | ~$39.99* |

Two clusters: **(A) tailoring/ATS** (Jobscan, Rezi, Kickresume, Teal) with high doc quality and no
auto-submit; **(B) volume/auto-apply** (LazyApply, Sonara, AIApply, Simplify, Jobright). Free tiers
are strongest where there's no automation; auto-apply monetizes hard via credits/subscriptions.

---

## 3. Common gaps across the field

1. **Volume ≠ quality.** Sonara applications "read identically" (3P); mass-bot output gets
   "rejected faster, at scale"; reported failure/submit rates 25–40% (Sonara, 3P).
2. **Cloud-only, data-hostile.** Every incumbent is SaaS; users upload resumes/IDs/work history,
   sometimes credentials. **No local/offline option exists.**
3. **No BYO-LLM-key.** AI is bundled at flat/credit prices with no model choice and no cost control.
4. **Billing traps.** 4-week cycles (Sonara ≈ 13 charges/yr), weekly tiers annualizing 2–3× monthly,
   auto-renewing trials, many tools skip annual (Teal, Jobscan, Simplify, Jobright).
5. **Opaque pricing / no trial / no refund** (Jobright, Simplify+, Sonara).
6. **Weak cross-platform coverage.** Mostly web + Chrome extension; **Firefox ignored/beta**, **Linux
   effectively unserved**, few native desktop apps.
7. **Lock-in.** Data lives in their cloud; cancel = lose access/data.
8. **Mass-apply ToS/reputation risk** (LazyApply requires Gmail; bots auto-submit) not disclosed up front.
9. **Black-box matching.** Almost none explain *why* a role scored high.
10. **Interview prep, negotiation, post-application CRM underbuilt** — where outcomes are actually won.
11. **Fabrication risk.** Jobright users reportedly see fabricated resume lines.

---

## 4. Unmet needs

- **Privacy-sensitive / high-profile seekers** (exec, gov/defense, legal, health) who cannot upload
  a resume to an unknown cloud.
- **Cost control** for power appliers (500+ roles) avoiding flat $40/mo or opaque credits.
- **Model neutrality** — users already paying OpenAI/Anthropic/Gemini/OpenRouter want to plug a key in.
- **Consent-first automation** — assist (rank + prefill) but **never a silent submit** in the user's name.
- **Cross-OS parity** — Windows/macOS **and Linux**, offline-capable.
- **Data portability** and **truthfulness guardrails** (no fabricated experience).
- **Explainable scoring** with a human-in-the-loop review/diff queue before submit.

---

## 5. Where a local-first desktop app wins

**Positioning:** *"Your job search runs on your machine. Your resume never leaves it. You pick the
model. Nothing is submitted without your click."* — the explicit inverse of all 11 incumbents.

| Incumbent weakness | Local-first desktop counter |
|---|---|
| Cloud-only data (all 11) | **Data residency by default**: resume, work history, LLM outputs in a local encrypted store (SQLite/Keychain); sync opt-in/E2E/absent — audit-friendly for regulated users |
| Bundled AI at flat/credit cost | **BYO-LLM key** (OpenAI/Anthropic/Gemini/OpenRouter/**local Ollama**); free tier runs local models. Undercuts AIApply's stacking and Jobright/Sonara credits |
| Silent auto-submit (LazyApply/Sonara/Jobright/AIApply) | **Consent-gated pipeline**: agent drafts + prefills, user reviews a diff, one click to submit |
| Web/Chrome-only; Linux/Firefox ignored | **Native Windows/macOS/Linux** app (Tauri/Electron) + optional local browser bridge; works offline |
| Billing traps | **Local license / one-time or transparent annual**; no weekly tiers or auto-renewing trials |
| Lock-in; data lost on cancel | **Files are yours** — export Markdown/PDF/CSV/JSON anytime; job DB is a local file |
| Black-box matching | **Explainable on-device scoring** (keyword + embedding similarity), user-editable weights |
| Fabrication risk | **Truthfulness guardrails** — rewrite only extracted facts; flag unverifiable claims |
| Interview-prep gap | **Local mock interview** with the user's chosen LLM; no per-tool add-on |
| Scraping/ToS risk | **Sanctioned read layer** (ATS boards + free feeds) + user-supplied postings |

**Beachhead segments:** privacy-sensitive professionals; EU/GDPR-sensitive users (Kickresume proves
demand); cost-conscious power appliers; Linux/offline users; AI users who already hold an API key.

**What we cannot beat them on (be honest):** breadth of ATS autofill coverage (Simplify/Jobright
invest heavily), listing freshness/aggregation, and turnkey zero-setup UX. **Winning move:**
interoperate — rank, tailor, and explain locally, then hand off to the user's existing free
autofill extension for form-fill. Own the private, judgment-heavy layer; let incumbents keep the
commodity autofill.

---

## Sources (accessed 2026-10-04)

Primary: `huntr.co/pricing`; `lazyapply.com/pricing`; `kickresume.com/en/pricing`; `rezi.ai/pricing`;
`simplify.jobs` + `help.simplify.jobs/a/5623502`; `support.aiapply.co/a/15692812` & `/a/14216708`;
`careerflow.ai/premium` + `/faq` + `help.careerflow.ai/a/10605570`; `jobscan.co` / `app.jobscan.co/plan`;
`tealhq.com/pricing` (403) + `help.tealhq.com/a/9530153` & `/a/9535168`; `sonara.ai/pricing` (intermittent);
`jobright.ai`.
Third-party (used where primary unavailable, flagged): `applyarc.com`, `itechguides.com`,
`frontdeskreview.com`, `resumly.ai`, `remotejobassistant.com`, `noxjobs.com`, `outapply.com`,
`zplatform.ai`, `hiredradar.com`, `applyall.com`, `fastapply.co`, `usesprout.com`, `applybolt.app`, `stork.ai`.

**Confidence flags:** Sonara and Jobright pricing **[UNVERIFIED/conflicting]**. Teal pricing
corroborated but direct fetch blocked. AIApply free-tier specifics partially unverified. Simplify
"Autopilot" auto-submit unverified.
