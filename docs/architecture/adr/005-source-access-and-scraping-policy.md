# ADR-005 — Source access classes, scraping and auto-apply policy

- **Status:** Accepted · Approved by Diego 2026-10-04 (§17 Q5) · 2026-10-04
- **Spine:** AD-9, AD-10, AD-11, AD-12

## Context
Locked: official and aggregator APIs, RSS and public ATS JSON by default. User-session browser automation is allowed only behind an explicit per-source opt-in with a ToS-risk warning. The orquestador's first policy (2026-10-04, **superseded the same day**) made auto-apply ON-capable only for public ATS forms and sanctioned APIs, and put session-board auto in v1.1. Diego then moved LinkedIn Easy Apply and Indeed auto-apply into the MVP (§17 Q5), still only behind an explicit per-source opt-in with an account-ban/ToS warning, conservative rate limits, daily caps and a global kill switch. The research (`integrations.md`) shows that LinkedIn (User Agreement §8.2), Indeed and WeWorkRemotely prohibit automated applying, and that InfoJobs is the only sanctioned candidate-side apply API.

## Decision
1. **Access classes** declared in each adapter manifest: `official_api | public_json | rss | html_scrape | session_browser | external_process`. Each manifest also declares a **legal tier** (`safe | caution | prohibited_automation`) and an **apply capability** (`none | link_out | ats_form | sanctioned_api | email | session_form`).
2. **Default on:** `public_json` and `rss` sources, plus `official_api` sources once a key or OAuth connection exists. **Opt-in required:** `html_scrape`, `session_browser`, `external_process`. The opt-in records the source, the timestamp and the warning version, and it is audited. For `prohibited_automation` sources it is re-confirmed every 30 days.
3. **One HttpClient:**
   - per-host token buckets
   - manifest limits (for example, Remotive ≤ 4 fetches/day)
   - robots.txt for HTML fetches
   - honest User-Agent
   - backoff on 429 and 5xx only
4. **No evasion, ever:** no CAPTCHA solving, fingerprint spoofing, proxy rotation or fake accounts. A CAPTCHA or login wall downgrades the step to assist mode.
5. **Session browser:** the user's own Chrome, Edge or Chromium, driven by chromiumoxide (CDP), with a dedicated Boba profile, always headed, at human-scale pacing. CDP runs over a pipe or an ephemeral `127.0.0.1` port, cookie protection is disclosed per platform, and each source has a sign-out (architecture §6.4).
6. **Apply policy:** modes `off | assist | auto` per source.
   - `auto` is available for `ats_form`, `sanctioned_api` and `email`.
   - For `session_form` sources (always `prohibited_automation`), `auto` needs the source's opt-in, the PRD FR-91 limits (default 3/day, ≥10 min pacing, ≥60 min Outbox hold, 7-day auto-pause on a challenge) and the 30-day re-confirmation.
   - Auto-send requires: threshold met, confidence met, no FLAG or VETO, verified documents, caps not exhausted, kill switch off. The kill switch is re-checked before every irreversible step.
7. **Audit:** a hash-chained, append-only `audit_event` row before and after every outbound action.

## Consequences
- The MVP is legally conservative by default and still meets the "full auto-apply available" requirement.
- Session-board automation may still get the user's account restricted. The warning says so plainly, and the risk sits with the user's explicit choice.
- Scope (Q5, decided by Diego 2026-10-04): session-board auto-apply (LinkedIn Easy Apply, Indeed) ships **in MVP**, per-source opt-in with ban/ToS warning, caps, kill switch and audit log, executed only by the core apply engine, never by LLM tools. LATAM session boards (Computrabajo, Bumeran…) and InfoJobs follow in v1.1. The research recommendation "do not build" (`integrations.md`) is overridden by this product decision. The residual account-ban risk is user-accepted and re-confirmed every 30 days.

## Alternatives
- **Playwright bundled with browsers.** Adds Node and about 150 MB of browsers per OS. See tech-stack.md.
- **Tauri webview automation.** Mixes untrusted third-party origins into the app's privileged webview, WebKitGTK is often blocked by boards, and behaviour differs by OS. Rejected.
- **No automation at all.** Contradicts the locked decision.
