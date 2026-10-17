# Gmail reading with your own Google client (S8.5, FR-34)

Boba's bundled Google client asks for `gmail.send` and nothing else: `gmail.readonly`
is a Google **restricted** scope, and reading with a bundled client needs a CASA
security assessment. Until that assessment exists, reading Gmail through the Gmail
API is only available with **your own** Google OAuth client ("BYO"), which keeps the
consent, the quota and the review obligation on your own project. Reading with the
bundled client stays deferred; IMAP and `.eml` import remain the alternatives.

## Setting up the BYO client

1. In the Google Cloud console, create (or reuse) a project and set its OAuth
   consent screen.
2. Create an OAuth client of type **Desktop app**. Copy its client ID and the
   client secret Google prints next to it (that value is public for installed
   apps; Boba only sends it to Google's token endpoint).
3. Enable the **Gmail API** for that project.
4. In Boba: Connections → Gmail → choose **Read and send**, paste the client ID and
   the client secret, and sign in. Boba asks for `gmail.send` and
   `gmail.readonly` and nothing else.

## Accepted behaviour (documented, not bugs)

- **Testing status expires refresh tokens after 7 days.** While the OAuth consent
  screen is in "Testing", Google issues refresh tokens that die after seven days,
  so the next daily run after a week asks you to reconnect. Publish the app (or
  move the consent screen to "In production") to stop that. Boba surfaces it as
  the ordinary "authorise again" state; nothing is lost, and reconnecting keeps
  your data.
- **Messages are fetched one by one.** The Gmail API lists ids before it hands
  over a message, and the reader fetches each chosen message separately (up to a
  page of 100 per pass). That is slower than Graph's single call and bounded by
  the same per-pass limit; a large backlog is worked through over several passes.
- **Reading needs the read+send mode and your own client.** A send-only Gmail
  connection, or one on Boba's bundled client, is refused by the reader itself
  (not merely hidden in the UI).

## Pacing and failure handling

- A `401` renews the access token once and repeats the request; a second `401` is
  reported as "authorise again".
- A `429` or a `5xx` is retried up to three times: the pause is the provider's
  `Retry-After` (capped at 30 s) or 250 ms doubling. The last status is reported
  when the retries run out.
- A pass reads the **oldest** messages of its window (Gmail lists newest-first and
  has no ascending order), following `nextPageToken` to the end of the listing. If
  the listing never reports its end, the pass reads nothing and logs a warning, so
  the sync cursor never moves past mail that was not looked at.
