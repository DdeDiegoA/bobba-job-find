# Boba Job Find — Voice & Tone

> **Status:** final draft · **Owner:** ux · **Date:** 2026-10-05 (P2-14b: conditional reply-reading copy)
> Companions: [`brand.md`](./brand.md) · [`DESIGN.md`](./DESIGN.md) · [`EXPERIENCE.md`](./EXPERIENCE.md)
> **Vocabulary is binding** per `docs/product/PRD.md` §3. UI terms in this document match that
> glossary (Today, Inbox, Studio, Tracker, Profile, Connections, Settings · Fit Score, Quality Score,
> Rank Score, Verdict, Confidence, Explanation · Application, Stage, Application Package, Document,
> Template, Apply Mode, Auto Rule, Outbox, Kill Switch, Audit Log).

---

## 1. The voice in one line

**A warm, honest job-search coach who happens to make great boba.**

The product does the tedious, repetitive scanning so the human keeps the decisions. The voice
*encourages effort*, never promises outcomes. It is warm on the surface and straight with the facts.

## 2. Voice attributes (always true)

| Attribute | Sounds like | Never sounds like |
|---|---|---|
| **Warm** | "Nice — you're set up." | "CONGRATULATIONS!!!" |
| **Encouraging** | "This one's a reach — but your Go work is a strong match." | "You're not qualified." |
| **Honest** | "Fit Score 38 — the seniority gap is real." | "Great opportunity!" (when it isn't) |
| **Calm** | "Nothing new today. That's normal." | "URGENT: 47 jobs expiring!" |
| **Human** | "We couldn't reach the board." | "Error 503: upstream failure." |
| **Yours** | "Stays on your machine." | "Synced to the cloud." |

## 3. Principles

1. **Coach, not cheerleader.** Encourage the *effort* (you applied, you showed up); never guarantee
   the *result*. No streaks, no "you got this!" for a role you can't win.
2. **Name the gap, then the path.** Bad news always arrives with a next step. "This one's a reach —
   you're missing Kubernetes — but your Go work is a strong match. Apply anyway or save it?" (PRD §7)
3. **Both scores, plain words.** **Fit Score** = can you get it. **Quality Score** = do you want it.
   The **Verdict** comes from the **Rank Score**. Say it like a human.
4. **The user decides.** Deferential, never pushy. "Your call." Overrides are always present.
5. **Warmth in surfaces, rigor in numbers.** Never color a poor Verdict as good. Honesty beats comfort.
6. **Plain language.** No ATS jargon, no recruiter-speak, no "leverage", no "utilize".
7. **Second person, active voice.** "You saved 3 Offers." Not "3 Offers were saved."
8. **Local-first, said out loud.** "On your computer", "nothing leaves this machine" — quiet trust.
9. **Short.** One idea per sentence. The user is job-hunting; they're tired already.
10. **Documents are not the app's voice.** Generated CVs and cover letters use the user's own
    professional voice (their writing sample), never the Boba voice (PRD §7).

## 4. Vocabulary

**Use → Avoid**

| Use (binding term) | Avoid | Why |
|---|---|---|
| Offer | "lead", "prospect", "listing" | PRD term |
| Fit Score / Quality Score / Rank Score | "match %", "compatibility index" | two scores + a rank |
| Verdict (Strong/Good/Moderate/Weak/Poor) | "rating", "grade" | band of the Rank Score |
| a reach / a stretch | "poor match", "unqualified" | kind, actionable |
| your Profile | "candidate record" | it's theirs |
| **Save** / **Dismiss** | "bookmark" / "skip", "reject" | Inbox actions |
| **Send** (Review mode) | "submit candidacy" | plain |
| **Auto** (Apply Mode) | "hands-free", "unattended automation" | the mode is Review or Auto |
| the Outbox | "send queue" | binding term |
| Kill Switch | "stop button" | binding term |
| nothing new today | "0 results found" | normalizes a quiet day |
| we couldn't reach… | "failed", "error occurred" | no blame, no jargon |

**Never:** emoji as UI icons; exclamation stacking; ALL CAPS urgency; "hurry"; countdowns;
"Don't miss out"; guilt ("You haven't applied in 3 days"); gambling language ("win", "jackpot").

**Tone intensity** — the same voice, adjusted:

```
routine ─────────────────────────────────────────── high-stakes
empty states · settings        application sent      rejection · error · veto · Kill Switch
(light, playful)               (proud, concise)      (calm, kind, grounded)
```

## 5. Microcopy

> EN is the source string; ES is a first-class translation (tú form, neutral ES/LATAM), not a literal
> gloss. `{braces}` are variables. ES runs ~15–30% longer; no string depends on a fixed width.

### 5.1 Onboarding

Order follows PRD FR-1…FR-10.

> **Reply-reading copy is always conditional (FR-34).** Default Gmail is **send-only**; reply reading
> exists only via Outlook/Graph, IMAP/.eml, or the user's own OAuth client ID. Every string that
> mentions reading replies must branch on the Connection's read capability — never promise replies
> unconditionally. Send-only variant says where replies *actually* go (the user's inbox).

| Beat | EN | ES |
|---|---|---|
| Welcome title | **Let's find work worth applying to.** | **Busquemos trabajos que valgan la pena.** |
| Welcome body | We'll set up your AI engine, Profile and Sources. It all stays on your computer. | Vamos a configurar tu motor de IA, tu perfil y tus fuentes. Todo queda en tu computadora. |
| Language & theme | Choose your language. The theme follows your system. | Elige tu idioma. El tema sigue el de tu sistema. |
| Agent CLI — found | Nice — we found **{n}** Agent CLIs on your machine. Pick one to run your searches. | Bien: encontramos **{n}** CLIs de agente en tu equipo. Elige una para tus búsquedas. |
| Agent CLI — none | No Agent CLI found yet. Here's how to install one — or continue in **Limited mode**. | Aún no encontramos ninguna CLI de agente. Aquí te decimos cómo instalar una, o continúa en **modo limitado**. |
| Test call OK | Works — {model}, {latency} ms. | Funciona: {modelo}, {latencia} ms. |
| Test call fail | That didn't work: {error}. Hint: run `{cli}` once to sign in. | No funcionó: {error}. Pista: ejecuta `{cli}` una vez para iniciar sesión. |
| Profile import | Drop your Master CV (PDF, DOCX) or a LinkedIn export. We'll read it and fill in your Profile. | Suelta tu CV base (PDF, DOCX) o tu export de LinkedIn. Lo leemos y llenamos tu perfil. |
| Import success | Got it — we pulled out **{n}** skills. Nothing is saved until you confirm each field. | Listo: extrajimos **{n}** habilidades. Nada se guarda hasta que confirmes cada campo. |
| Profile gaps | Three quick things the CV couldn't tell us: work eligibility, your English level, and your **Salary Floor**. | Tres cosas que el CV no nos dijo: autorización de trabajo, tu nivel de inglés y tu **salario mínimo**. |
| Sources | Start with the recommended Safe Sources. Add your own RSS or URL anytime. | Empieza con las fuentes seguras recomendadas. Agrega tu propio RSS o URL cuando quieras. |
| Email (optional) | Connect your email so we can send applications. Gmail sends only by default — reply reading needs Outlook/Graph, IMAP, or your own OAuth client ID. Skip if you'd rather not. | Conecta tu correo para enviar postulaciones. Gmail por defecto solo envía; para leer respuestas necesitas Outlook/Graph, IMAP o tu propio ID de cliente OAuth. Puedes omitirlo si prefieres. |
| Apply Mode & schedule | Default is **Review** — you approve every send. You can set a daily run time. | Por defecto es **Revisión**: tú apruebas cada envío. Puedes fijar una hora diaria. |
| Recovery backup — title (P0-1) | **Save your recovery key** | **Guarda tu clave de recuperación** |
| Recovery backup — why | This build isn't signed for every update. If one ever locks Boba out of your data, this key is how you get back in. Keep it safe — it's the only copy; Boba can't recover it for you. | Esta versión no está firmada para todas las actualizaciones. Si alguna vez Boba no puede abrir tus datos, esta clave te devuelve el acceso. Guárdala bien: es la única copia; Boba no puede recuperarla por ti. |
| Recovery key label / actions | Recovery key · Copy · Print · Save backup file | Clave de recuperación · Copiar · Imprimir · Guardar copia de seguridad |
| Recovery backup — ack | ☑ I've saved my recovery key somewhere safe | ☑ Guardé mi clave de recuperación en un lugar seguro |
| Recovery backup — can't skip | This step is required on this build — it's how you stay in control of your data. | Este paso es obligatorio en esta versión: así mantienes el control de tus datos. |
| Recovery screen (failed update) | **Restore from backup** — paste your recovery key to open your Boba data. | **Restaurar desde una copia** — pega tu clave de recuperación para abrir tus datos de Boba. |
| Recovery restore — success | Restored. Everything is back on your machine. | Restaurado. Todo volvió a tu equipo. |
| Recovery restore — wrong key | That key didn't match this backup. Check for typos — nothing was lost. | Esa clave no coincide con esta copia. Revisa si hay errores: no se perdió nada. |
| First run | All set. Let's run your first search. | Todo listo. Hagamos tu primera búsqueda. |
| First run — running | Steeping… scanning **{n}** Sources. This runs on your machine. | Preparando… revisando **{n}** fuentes. Esto corre en tu equipo. |
| Limited mode note | Search, scoring and the Tracker work without AI. Tailoring and profile reading need an Agent CLI. | La búsqueda, el puntaje y el seguimiento funcionan sin IA. Adaptar y leer el perfil necesita una CLI de agente. |

### 5.2 Empty states

| Surface | EN | ES |
|---|---|---|
| Today — first run not done | **Nothing brewed yet.** Run your first search and your matches land here. → *Run search* | **Aún no hay nada listo.** Haz tu primera búsqueda y tus coincidencias aparecerán aquí. → *Buscar ahora* |
| Today — ran, 0 new | **Nothing new today.** That's normal — new roles post all week. We'll keep watch. | **Hoy no hay novedades.** Es normal: se publican ofertas toda la semana. Seguimos atentos. |
| Inbox — filters return nothing | No Offers for **{filters}**. Try loosening one. → *Clear filters* | Sin ofertas para **{filtros}**. Prueba quitar un filtro. → *Limpiar filtros* |
| No Sources connected | **No Sources yet.** Connect a job board or paste a link to get started. | **Aún no hay fuentes.** Conecta un portal o pega un enlace para empezar. |
| Tracker — empty Stage | Nothing here yet. Move a card in, or tailor an Offer to start it. | Aquí no hay nada todavía. Mueve una tarjeta o adapta una oferta para empezar. |
| Inbox — nothing saved | **No saved Offers yet.** Tap **Save** on an Offer to keep it for later. | **Aún no guardaste ofertas.** Toca **Guardar** en una oferta para verla después. |
| Email — no replies (read on) | No replies yet. Recruiters often take a week — we'll bring them to you. | Sin respuestas aún. Los reclutadores suelen tardar una semana; te avisamos. |
| Email — send-only (no read) | This connection sends only, so replies land in your inbox, not here. Enable read via Outlook/Graph, IMAP or your own client ID to track them. | Esta conexión solo envía, así que las respuestas llegan a tu correo, no aquí. Activa la lectura con Outlook/Graph, IMAP o tu propio ID de cliente para seguirlas. |
| Outbox — empty | The Outbox is empty. Automatic sends wait here before they go out. | La bandeja de salida está vacía. Los envíos automáticos esperan aquí antes de salir. |

### 5.3 Digest notification (OS + tray)

| Beat | EN | ES |
|---|---|---|
| Title (has news) | **{strong} Strong, {good} Good today** | **{strong} altas, {good} buenas hoy** |
| Title (none) | **Nothing new today** | **Hoy sin novedades** |
| Body | Top: {job} · {company} — Fit {fit}, Quality {quality} | Top: {puesto} · {empresa} — Encaje {fit}, Calidad {calidad} |
| Body (Outbox) | {n} waiting in the Outbox — review or let them send at {time}. | {n} esperando en la bandeja de salida: revísalas o déjalas enviar a las {hora}. |
| Body (needs you) | {n} need your approval. | {n} necesitan tu aprobación. |
| Body (0 new) | We checked {n} Sources. Nothing worth your time — yet. | Revisamos {n} fuentes. Nada que valga tu tiempo… por ahora. |
| Actions | Open · Run now | Abrir · Buscar ahora |
| Tray — running | Steeping {n} of {m} Sources… | Preparando {n} de {m} fuentes… |
| Tray — done | Last run {time} · {n} new | Última búsqueda {hora} · {n} nuevas |
| Tray — paused | Schedule paused. Nothing runs until you resume. | Programación en pausa. Nada corre hasta que reanudes. |
| Tray — Kill Switch on | All automatic sending stopped. | Todos los envíos automáticos detenidos. |
| Tray — locked (P1-5) | Boba is locked — enter your passphrase to run. → Unlock | Boba está bloqueado: ingresa tu contraseña para buscar. → Desbloquear |
| Notification — locked run (P1-5) | **Boba is locked.** Today's search was skipped. Enter your passphrase to run it. | **Boba está bloqueado.** La búsqueda de hoy se omitió. Ingresa tu contraseña para hacerla. |
| Tray — error | Couldn't reach {source}. Everything else ran fine. → Retry | No pudimos conectar con {fuente}. El resto funcionó bien. → Reintentar |

**Notification rules:** at most one digest per Search Run (FR-45). Respect OS "do not disturb" and
quiet hours. No notification on the first run until onboarding is finished. No urgency words.

### 5.4 Low-fit Offer — "a reach — here's why"

The signature honesty moment. **Never hide it, never color it green.** Lead with the truth, then the
path, then the choice.

**EN**
> **This one's a reach — and here's why.**
> Fit Score 38, Verdict Weak. You're mid-level against a senior role, and 2 of 5 must-haves aren't in
> your Profile yet: {missing skills}. Quality Score is strong (78) — the company and pay are worth it.
> You can still apply, or save it for when you've shipped {missing skill}. Your call.

**ES**
> **Esta es ambiciosa, y te decimos por qué.**
> Encaje 38, veredicto Bajo. Eres de nivel medio y el puesto es senior; 2 de 5 requisitos clave aún no
> están en tu perfil: {habilidades faltantes}. La calidad es alta (78): la empresa y el salario valen
> la pena. Puedes postularte igual, o guardarla para cuando domines {habilidad faltante}. Tú decides.

**Rules**
- Name the score and the Verdict plainly. Show the numbers.
- Give the *why* in at most two lines; the Explanation is available on expand.
- Always offer a path and a choice (apply anyway / save for later). No dead ends.
- When Quality is high and Fit low (or the reverse), say both — that is the point of two axes.

### 5.5 Auto (Apply Mode) confirmations

Enabling **Auto** is a high-stakes action: dialog + explicit confirmation + per-Source opt-in
(PRD FR-83, FR-86, FR-91). **Review** is the default.

| Beat | EN | ES |
|---|---|---|
| Enable dialog title | **Turn on Auto apply?** | **¿Activar la postulación automática?** |
| Enable body | Boba will send applications that match an Auto Rule, up to {limit} a day, after a {hold} Outbox hold. You can stop it anytime with the Kill Switch. | Boba enviará postulaciones que cumplan una regla automática, hasta {límite} por día, tras una espera de {hold} en la bandeja de salida. Puedes detenerlo con el interruptor de emergencia. |
| Confirm checkbox | I understand Auto sends real applications in my name. | Entiendo que el modo automático envía postulaciones reales a mi nombre. |
| Dry run | This rule's first **{n}** matches run as a dry run — we'll show what we *would* send, without sending. | Las primeras **{n}** coincidencias de esta regla serán una prueba: te mostramos qué *enviaríamos*, sin enviar nada. |
| Auto Rule summary | Rule: Verdict ≥ {verdict}, Fit ≥ {fit}, Confidence ≥ {confidence}, {cap}/day, {company} per company. | Regla: veredicto ≥ {veredicto}, encaje ≥ {fit}, confianza ≥ {confianza}, {tope}/día, {empresa} por empresa. |
| Outbox item | Waiting {countdown}. [Review now] [Cancel] | Esperando {cuenta}. [Revisar ahora] [Cancelar] |
| Send (Review, read on) | Sent to {company}. We'll watch for a reply. | Enviada a {empresa}. Estaremos atentos a su respuesta. |
| Send (Review, send-only) | Sent to {company}. Replies will land in your inbox — this connection can't read them yet. | Enviada a {empresa}. Las respuestas llegarán a tu correo; esta conexión aún no puede leerlas. |
| Run summary | **Run done.** {sent} sent automatically, {held} waiting in the Outbox, {review} need you. | **Búsqueda completada.** {sent} enviadas automáticamente, {held} en la bandeja de salida, {review} necesitan tu revisión. |
| Needs check | We couldn't confirm the {company} send. It's marked **Needs check** — open it to verify. | No pudimos confirmar el envío a {empresa}. Está marcado **Revisar**: ábrelo para verificarlo. |
| Kill Switch confirm | **Stop all automatic sending?** Items in the Outbox move to your review queue. | **¿Detener todos los envíos automáticos?** Lo que está en la bandeja de salida pasa a tu cola de revisión. |
| Kill Switch toast | Kill Switch on. Nothing sends automatically until you turn it back off. | Interruptor de emergencia activo. No se envía nada automáticamente hasta que lo desactives. |
| Session Board opt-in | Auto on **{source}**? This can risk your account and breaks some sites' rules. Type **{source}** to confirm. | ¿Modo automático en **{fuente}**? Puede arriesgar tu cuenta e incumplir las reglas de algunos sitios. Escribe **{fuente}** para confirmar. |

### 5.6 Errors

Calm, specific, non-blaming, one recovery action. No stack traces, no bare error codes, no mascot.

| Situation | EN | ES |
|---|---|---|
| Source unreachable | We couldn't reach **{source}** just now. Your other Sources ran fine. → Retry | No pudimos conectar con **{fuente}** ahora. Tus demás fuentes funcionaron bien. → Reintentar |
| Agent CLI not found | We couldn't start **{cli}**. Is it installed and on your PATH? → Choose another engine | No pudimos iniciar **{cli}**. ¿Está instalada y en tu PATH? → Elegir otro motor |
| Agent quota / auth | **{cli}** needs attention: {reason}. AI tasks are paused; the rest of the run continues. → Fix | **{cli}** necesita atención: {motivo}. Las tareas de IA están en pausa; el resto continúa. → Arreglar |
| Master CV parse failed | We couldn't read that file. Try a PDF or DOCX, or paste your CV as text. | No pudimos leer ese archivo. Prueba con PDF o DOCX, o pega tu CV como texto. |
| Email auth expired | Your **{provider}** Connection needs a fresh sign-in. → Reconnect | Tu conexión con **{provider}** necesita que inicies sesión de nuevo. → Reconectar |
| Rate-limited | **{source}** asked us to slow down. We'll try again in {time}. | **{fuente}** nos pidió bajar el ritmo. Reintentaremos en {time}. |
| Draft blocked (no-fabrication) | A claim in this Document isn't in your Profile: "{claim}". Fix it or confirm it to add it. | Una afirmación de este documento no está en tu perfil: «{claim}». Corrígela o confírmala para agregarla. |
| ATS check failed | The PDF text layer has a problem, so Auto won't send it. You can still send manually. | El texto del PDF tiene un problema, así que el modo automático no lo enviará. Puedes enviarlo a mano. |
| Offline | You're offline. Search and tailoring need a connection; your data is still here. | Estás sin conexión. La búsqueda y la adaptación necesitan internet; tus datos siguen aquí. |
| Generic/unexpected | Something went wrong on our side. Your data is safe. → Try again | Algo falló de nuestro lado. Tus datos están seguros. → Reintentar |

### 5.7 Rejections (empathetic)

Distinguish a **recruiter rejection** (external, sensitive) from a **self-dismissed Offer** or
**Withdrawn** Application (user-initiated, neutral). Status changes arrive as **Status Proposals**
that the user approves (FR-102).

| Beat | EN | ES |
|---|---|---|
| Status Proposal — rejection | **Update on {company}:** they've moved forward with other candidates. That happens — and it isn't a verdict on you. [Approve change] | **Novedad de {empresa}:** avanzaron con otras personas. Pasa, y no es un juicio sobre ti. [Aprobar cambio] |
| Proposal body (gentle) | You gave it a real shot. Want to note why this one felt off, for next time? | Le diste una oportunidad de verdad. ¿Quieres anotar por qué no encajó, para la próxima? |
| After rejection — actions | Save this company for later · Find similar Offers · Take a break | Guardar esta empresa · Buscar ofertas similares · Tomarte un respiro |
| Dismiss confirm | Dismiss **{offer}**? It leaves your Inbox — you can undo. | ¿Descartar **{oferta}**? Sale de tu bandeja, pero puedes deshacerlo. |
| Cluster of rejections | **A few nos in a row is normal.** Want to pause the schedule for today? | **Varios rechazos seguidos es normal.** ¿Quieres pausar la programación por hoy? |
| Withdrawn (self) | Marked **Withdrawn**. Nothing else happens. | Marcada como **Retirada**. No pasa nada más. |

**Rules:** never "unfortunately" (shifts blame); never a celebration animation on a rejection; never
nudge to apply more after one. A cluster of rejections **offers a break**, never pressure.

### 5.8 Success moments

| Beat | EN | ES |
|---|---|---|
| First matches | **Your first pearls are ready.** {n} Offers to look at. | **Tus primeras perlas están listas.** {n} ofertas para revisar. |
| Application sent | Sent to {company}. Nice work. | Enviada a {empresa}. Buen trabajo. |
| Interview invite | **Interview with {company}.** You earned this one. | **Entrevista con {empresa}.** Este lo conseguiste tú. |
| Offer received | **An offer from {company}.** Take your time with this one. | **Una oferta de {empresa}.** Tómate tu tiempo con esta. |
| Milestone | {n} Applications out. Momentum. | {n} postulaciones en marcha. Buen ritmo. |

**Rules:** celebrate **effort and progress**, not luck or the algorithm. Never imply an offer is
guaranteed. "You earned this one" references their work, not the app's magic.

### 5.9 Profile, proposals & diagnostics

| Beat | EN | ES |
|---|---|---|
| Import review | We read **{n}** fields from your Master CV. Check each one — nothing is saved until you confirm. | Leímos **{n}** campos de tu CV base. Revisa cada uno: nada se guarda hasta que confirmes. |
| Field evidence | Evidence: “{excerpt}” | Evidencia: «{excerpt}» |
| Variant missing | This field has no {ES} version yet. Draft it, then review before it can be sent. | Este campo aún no tiene versión en {ES}. Redáctala y revísala antes de poder enviarla. |
| Version saved | Saved as version {n}. Open Offer scores will refresh on the next run. | Guardado como versión {n}. Los puntajes se actualizarán en la próxima búsqueda. |
| Restore version | Restore version {n}? Your current version is kept in the history. | ¿Restaurar la versión {n}? Tu versión actual queda en el historial. |
| Status Proposal (interview) | **Update on {company}:** “{quoted line}” → move to **Interviewing**? | **Novedad de {empresa}:** «{línea citada}» → ¿mover a **En entrevistas**? |
| Compare vs Applied | This looks like a **job offer**, not just a reply — review before you accept anything. | Esto parece una **oferta de trabajo**, no solo una respuesta; revísalo antes de aceptar nada. |
| Red-flag badge | ⚑ Likely ghost — posted {days} days ago. Deprioritised, not deleted. | ⚑ Posible oferta fantasma: publicada hace {días} días. Baja prioridad, no se elimina. |
| Fraud badge | ⛔ Possible recruitment fraud. **Auto is blocked** for this Offer — check it yourself. | ⛔ Posible fraude de reclutamiento. **El modo automático está bloqueado** para esta oferta; revísala tú. |
| Limited mode | AI features are off — no Agent CLI connected. Search, scoring and the Tracker still work. | Las funciones de IA están apagadas: no hay CLI de agente. La búsqueda, el puntaje y el seguimiento siguen funcionando. |
| Run report empty | No run yet today. The next one is at {time}. | Aún no hay búsqueda hoy. La próxima es a las {hora}. |
| Audit Log empty | Nothing logged yet. Sends, Auto decisions and Stage changes appear here. | Aún no hay registros. Los envíos, decisiones automáticas y cambios de etapa aparecerán aquí. |
| Network activity empty | Nothing contacted recently. Boba only calls what you enabled. | Sin conexiones recientes. Boba solo contacta lo que tú activaste. |

### 5.10 Capability gating & locked store (P0-2 · P1-5)

**Unsupported CLI for a task (P0-2).** Some tasks read untrusted text (job postings, emails), so they
run only on an Agent CLI with **verified tool lockdown**. When a CLI can't do a task, say *why* in one
plain line and point to the fix — never offer "run anyway".

| Beat | EN | ES |
|---|---|---|
| Engine matrix — cell | Unsupported | No compatible |
| Engine matrix — reason | **{cli}** can't lock down its tools, so it can't read job postings or email. | **{cli}** no puede restringir sus herramientas, así que no puede leer ofertas ni correo. |
| Task blocked (Studio) | **Tailoring needs an agent with tool lockdown.** {cli} can't run this task. → Change CLI | **Adaptar necesita un agente con herramientas restringidas.** {cli} no puede hacer esta tarea. → Cambiar CLI |
| Task blocked (tooltip/short) | Not supported by {cli} — this task reads untrusted text. | {cli} no lo admite: esta tarea lee texto no confiable. |
| No override | Boba won't run this on a CLI it can't sandbox. Pick another CLI. | Boba no hará esto en una CLI que no puede aislar. Elige otra CLI. |
| All CLIs unsupported | None of your Agent CLIs can run this safely. Install one with tool lockdown — or keep using Limited mode. | Ninguna de tus CLIs puede hacer esto de forma segura. Instala una con herramientas restringidas, o sigue en modo limitado. |

**Locked store (P1-5).** With the passphrase path, the store is locked until unlocked once per login
session. Say it plainly, never pretend a run succeeded.

| Beat | EN | ES |
|---|---|---|
| Unlock dialog title | **Unlock Boba** | **Desbloquear Boba** |
| Unlock dialog body | Enter your passphrase to decrypt your data. It stays in memory for this login session only. | Ingresa tu contraseña para descifrar tus datos. Solo permanece en memoria durante esta sesión. |
| Unlock field label | Passphrase | Contraseña |
| Unlock action | Unlock | Desbloquear |
| Unlock success | Unlocked. Your data is available for this session. | Desbloqueado. Tus datos están disponibles durante esta sesión. |
| Wrong passphrase | That passphrase didn't work. Try again — nothing was lost. | Esa contraseña no funcionó. Inténtalo de nuevo; no se perdió nada. |
| Topbar chip | Locked — Unlock | Bloqueado — Desbloquear |
| Run skipped (report) | Search skipped — locked | Búsqueda omitida — bloqueado |
| Idle re-lock (info) | Locked again after {time} idle. Unlock to continue. | Se bloqueó tras {time} sin uso. Desbloquea para continuar. |
| Lock is not an error | Your data is safe — it's encrypted and waiting for you. | Tus datos están a salvo: cifrados y esperándote. |

**Rules:** the locked state is never an error and never a failure; it is *paused, waiting for you*.
Never mark a skipped run as successful and never suppress the notification silently.

---

## 6. Microcopy glossary (EN ↔ ES)

Canonical UI terms. Both languages use these exactly (names marked **binding** come from
`docs/product/PRD.md` §3 — do not synonymize).

| EN | ES | Notes |
|---|---|---|
| **Today** | Hoy | digest of the latest Search Run, needs-you items, Outbox |
| **Inbox** | Bandeja | ranked Offers and Offer detail |
| **Studio** | Estudio | tailoring per Application |
| **Tracker** | Seguimiento | kanban board and Application detail |
| **Profile** | Perfil | |
| **Connections** | Conexiones | Sources + email accounts |
| **Settings** | Ajustes | |
| **Offer** | Oferta | a job posting (binding) |
| **Fit Score** | Puntaje de encaje / Encaje | can-you-get-it (binding; short: Encaje) |
| **Quality Score** | Puntaje de calidad / Calidad | do-you-want-it (binding) |
| **Rank Score** | Puntaje de posición | weighted total (binding) |
| **Verdict** | Veredicto | Strong/Good/Moderate/Weak/Poor → Alto/Bueno/Medio/Bajo/Muy bajo (binding) |
| **Confidence** | Confianza | High/Medium/Low → Alta/Media/Baja (binding) |
| **Lead** | Pista | low-Confidence presentation of a high Rank Score |
| **Explanation** | Explicación | factor breakdown + narration (binding) |
| **a reach** | ambiciosa / un reto | low-Fit framing |
| **Gap / missing skill** | Brecha / habilidad faltante | |
| **Matched skill** | Habilidad que coincide | |
| **Evidence** | Evidencia | quoted proof from the Offer |
| **Gate** | Filtro / verificación | PASS/FLAG/VETO |
| **Save** | Guardar | Inbox → creates an Application in Saved (binding Stage) |
| **Dismiss** | Descartar | removes from Inbox; optional reason; undoable |
| **Tailor** | Adaptar | opens the Studio |
| **Send** | Enviar | submits the Package in Review mode (binding) |
| **Apply Mode: Review** | Modo: Revisión | user approves each send (binding) |
| **Apply Mode: Auto** | Modo: Automático | rule-bound automatic sending (binding) |
| **Auto Rule** | Regla automática | thresholds + caps (binding) |
| **Outbox** | Bandeja de salida | hold queue for automatic sends (binding) |
| **Kill Switch** | Interruptor de emergencia | stops all automatic sending (binding) |
| **dry run** | prueba en seco | first N matches shown, not sent |
| **Apply Channel** | Canal de postulación | Email Apply / ATS Form / Sanctioned API / Session Board / Manual (binding) |
| **Application** | Postulación | pursuit of one Offer (binding) |
| **Application Package** | Paquete de postulación | Documents + answers; immutable once sent (binding) |
| **Document** | Documento | generated CV or cover letter (binding) |
| **Template** | Plantilla | visual layout (binding) |
| **Master CV** | CV base | the imported source CV (binding) |
| **Status Proposal** | Propuesta de estado | Stage change from an email, applied on approval (binding) |
| **Audit Log** | Registro de auditoría | append-only local record (binding) |
| **Stage** | Etapa | Saved/Preparing/Applied/Interviewing/Offer/Hired/Rejected/No Response/Withdrawn (binding) |
| **Saved** | Guardada | Stage |
| **Preparing** | En preparación | Stage |
| **Applied** | Postulada | Stage |
| **Interviewing** | En entrevistas | Stage |
| **Offer (Stage)** | Oferta | Stage — distinct from an Offer posting; disambiguate in copy |
| **Hired** | Contratada | Stage |
| **Rejected** | Rechazada | Stage |
| **No Response** | Sin respuesta | Stage |
| **Withdrawn** | Retirada | Stage |
| **Salary Floor** | Salario mínimo | binding term |
| **Search Run** | Búsqueda | one run across all Sources (binding) |
| **Run now** | Buscar ahora | |
| **Pause schedule** | Pausar programación | |
| **Source** | Fuente | binding; has Source Type + Risk Tier |
| **Risk Tier: Safe / Opt-in / Prohibited-automation** | Nivel de riesgo: Segura / Opcional / Automatización prohibida | binding |
| **Connection** | Conexión | binding |
| **Connect / Reconnect** | Conectar / Reconectar | |
| **Private / local** | Privado / local | "en tu equipo" |
| **Audit / Export data** | Auditoría / Exportar datos | |
| **Supported / Unsupported (task)** | Admite / No compatible | a task × CLI capability (P0-2); no user override |
| **tool lockdown** | herramientas restringidas | native shell + web removed; required to read untrusted text |
| **Locked / Unlock** | Bloqueado / Desbloquear | passphrase-derived store state (P1-5) |
| **Recovery key** | Clave de recuperación | one-time key to restore from backup (P0-1) |
| **Recovery backup** | Copia de recuperación | forced before first run on unstable-signing builds |
| **Theme: System/Light/Dark** | Tema: Sistema/Claro/Oscuro | |
| **Reduce motion** | Reducir movimiento | |
| **Shortcuts** | Atajos | |

**Punctuation & formatting**
- EN uses sentence case for UI labels ("Run search"), ES likewise ("Buscar ahora").
- No terminating period on short labels/buttons; full sentences in body copy.
- Numbers/dates/currency via `Intl` with the active locale (`1.234` vs `1,234`).
- Spanish: **tú**, not *usted*; no regional slang; avoid gendered defaults where a neutral phrasing
  exists ("la persona que recluta" over "el reclutador" when generic).
- Quotes around evidence are typographic: EN “ ”, ES « ».
