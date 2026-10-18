# FX5b review: offer your own client when the build has none bundled

- **Slice:** FX5b (branch `hermad/FX5b`)
- **Reviewed commit:** `4f3b091` (dev-1)
- **Reviewer:** reviewer-1
- **Verdict:** changes requested (one documentation fix; code approved)

## Gate (run in the slice worktree, `CARGO_INCREMENTAL=0`)

| Step | Result |
|---|---|
| `pnpm install --frozen-lockfile --ignore-scripts` | pass |
| `node scripts/gen-bindings.mjs --check` | pass (`bindings.ts` up to date) |
| `pnpm lint` (Biome) | pass, exit 0; 3 `noExplicitAny` warnings, all in generated `bindings.ts` helpers, not introduced by this slice |
| `pnpm typecheck` | pass |
| `pnpm test` (vitest) | pass, 649/649 |
| `check-i18n.mjs` | pass (EN/ES aligned, no unused keys) |
| `check-conventions.mjs` | pass |
| `check-workflow-policy.mjs` | pass |
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | pass |
| `cargo test --workspace --locked` | pass, 2152 passed, 0 failed |
| `cargo deny check` | pass (advisories, bans, licenses, sources ok) |

## Security review

- **No client id or secret crosses IPC or reaches the UI.** `email_bundled_clients`
  returns `BundledClients { outlook: bool, gmail: bool }` and nothing else. The test
  `the_probe_never_exposes_the_client_id_or_the_secret` asserts the serialised JSON
  contains none of the values. The `bindings.ts` diff adds only the command type.
- **No logging of the values.** The new Rust and TSX code writes no log lines.
- **Gmail needs both halves.** `gmail` is true only when the client id and the public
  secret are both present and non-blank. Blank values count as absent, which matches
  an empty CI secret.
- **Hidden one-click, BYO primary.** The onboarding step drops the button of an absent
  provider, and the address field is hidden when neither provider is available. The
  Connections forms make the own-client fields `required` and drop the "Advanced"
  label while the probe reports the provider absent. While the probe loads, the old
  behaviour is kept.
- **Guide scopes match the code.** Graph send-only is `Mail.Send offline_access`, and
  read+send adds `Mail.Read`. Gmail send-only is `gmail.send`, and read+send adds
  `gmail.readonly`. This matches `SCOPES_BY_MODE` and `GMAIL_SCOPES_BY_MODE` in
  `use-email.ts`, and S8.1, S8.2 and S8.5.
- **Loopback and PKCE are described correctly.** The code binds `127.0.0.1` on an
  ephemeral port and uses `/cb` (`oauth/loopback.rs`). It sends `code_challenge_method=S256`
  and a single-use `state` (`oauth/mod.rs`). The guide's description of these matches.
- **Gmail secret wording.** The guide and the field hint call the installed-app secret
  public, which matches RELEASING.md section 3 and residual R-S8.2-1.
- **7-day expiry.** The guide notes that Google's Testing status expires refresh tokens
  after seven days, which matches R-S8.5-1.
- **IMAP claim.** The guide says Boba tests both IMAP and SMTP before saving. This is
  true: `email_connect_imap_smtp` runs a probe before the account is saved (AC2).

## Findings

### F1 (medium, docs): Microsoft redirect URI in the guide does not match what Boba sends

`docs/user-guide/email-own-client.md`, Microsoft step 3, tells the user to register the
redirect URI `http://127.0.0.1`. Boba sends `http://127.0.0.1:<port>/cb`
(`oauth/loopback.rs:46`, `CALLBACK_PATH = "/cb"`). Entra ignores the port for loopback
redirects, but the path must still match. So a user who follows the guide literally
will hit a redirect-URI mismatch on the first attempt. The `/cb` variant is only given
as a fallback.

**Expected:** the primary instruction registers the exact URI Boba uses,
`http://127.0.0.1/cb`, and the troubleshooting line matches it.

### F2 (low, docs): an unclear and unverified data-handling claim

`docs/user-guide/email-own-client.md` line 29 says "Boba only reads messages that belong
to your applications, locally." The sentence is garbled and this review did not
verify it against the sync code. Reword it to what the code does, or remove it.

## Not in scope

- The 3 Biome warnings in generated `bindings.ts` predate this slice.

## Next step

dev-1 fixes F1 and F2 on `hermad/FX5b`, then re-emits `HERMAD:DONE story=FX5b` with a
new `n`. Reviewer-1 re-checks the two doc lines and re-runs the gate on the new commit.
