# Sources and connections

**Connections** is where you decide where Offers come from and which mailbox Boba may use.

## Sources

A **Source** is a place Boba reads job postings from. Each card shows its type, a **Risk
Tier**, when it last ran, how many Offers it found, and its health (OK, Degraded, Failing). A
Source that fails three runs in a row triggers a notification, so you're never guessing why
a board went quiet.

| Type | What it is | Examples |
|---|---|---|
| API / feed | Public APIs and feeds | Remotive, RemoteOK, We Work Remotely, Himalayas, Arbeitnow, Hacker News, Adzuna, Jooble |
| ATS board | A company's public applicant-tracking board | Greenhouse, Lever, Ashby, Workable, SmartRecruiters |
| Custom feed | Any RSS or Atom feed you add | your own |
| Manual | An Offer you paste in | a URL or the posting text |
| Session board | A board used through *your* signed-in browser | LinkedIn, Indeed (opt-in) |

Some sources need an API key or a company board name from you. The Connections screen asks for
what it needs.

### Add a custom feed

**Connections → Add a custom feed.** Paste the URL, press **Test fetch** to see sample items,
then **Save feed**. Boba checks it is a real feed before saving it.

### Add one Offer by hand

Found a posting somewhere Boba doesn't read? Add it by URL or pasted text and it is ranked like
any other Offer.

## Email

Connect a mailbox to send applications from your own address.

- **Outlook / Microsoft 365.** Boba sends you to Microsoft to sign in; it never sees your
  password. Choose **Send only**, or **Read and send** if you also want Boba to read replies
  about your applications.
- **Gmail.** Send-only by default. Reading Gmail replies works only with your own OAuth client
  ID (bring your own).
- **IMAP / SMTP.** For any other provider.

Sign-in happens in your browser and Boba accepts the answer only on your own machine
(`127.0.0.1`). If the browser window is closed or blocked, try again, or use your own client
ID or IMAP. **Disconnect** revokes access where the provider allows it and deletes the stored
tokens.

Reading replies is off unless you choose it. Boba reads only messages that belong to your
applications, locally, and turns them into **Status Proposals** that you confirm. You can also
import a single `.eml` by hand.

## Session boards: read this first

LinkedIn and Indeed have no public job API and forbid automated use in their terms. Boba can
still work with them, but only **through your own signed-in browser session, one Source at a
time, after you opt in**. The risk is yours, and it is real: **your account can be restricted or
banned.**

When you turn one on, Boba shows a warning that you must read and acknowledge. What to know:

- Boba opens its own browser profile (your own Chrome). Cookies for that session live in that
  profile, protected by your OS (Keychain, DPAPI or your Linux keyring), **not** by Boba's
  database encryption. On a Linux machine with no keyring, the protection is weaker, and Boba
  says so.
- Boba reads slowly, at a human-like pace. It never creates accounts, never signs in for you
  and never solves CAPTCHAs.
- A CAPTCHA, a sign-in wall, an "unusual activity" notice or a rate limit **pauses that
  Source** and notifies you. Resolve it yourself, then press **I've resolved it, resume**.
- Applying on a session board is in **Review** mode: Boba fills the form on screen, you read
  every field and press submit yourself.
- **Session Board Auto** is a *separate*, riskier opt-in. You type the board's name to confirm,
  and it runs under stricter limits: 3 applications a day by default (10 at most), at least 10
  minutes between them, an Outbox hold of at least 60 minutes, a 7-day pause after any
  challenge, and re-confirmation every 30 days. The Kill Switch stops it like everything else.
- **Sign out / clear session** deletes a Source's session and cookies at any time. **Reset
  everything** removes them too.

Not comfortable with that? Skip session boards. The safe Sources cover a lot of ground, and you
can always add an Offer by hand.

Next: [Ranking](ranking.md).
