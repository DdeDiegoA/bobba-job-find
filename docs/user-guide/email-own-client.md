# Connect email with your own OAuth client

Some Boba builds ship without a bundled Microsoft or Google client, so the one-click
buttons have nothing to sign in with. This guide walks you through creating **your own**
OAuth client in a few minutes and pasting it into **Connections → Email**. Everything stays
on your machine: Boba stores the tokens in your OS keychain and never sends them anywhere
except the provider you choose.

You only need the entry for the provider you use. If creating a client is not for you, the
**IMAP / SMTP** form below the two OAuth forms connects almost any mailbox with an app
password, and needs no client at all.

## Before you start: how Boba signs in

- Boba opens your **system browser** and receives the answer on a **loopback** address on
  your own machine: `http://127.0.0.1:<random port>/cb`. The port is different every time,
  so there is no fixed port to register.
- The flow uses **PKCE (S256)** and a single-use `state`; there is no client secret that
  Boba treats as confidential (Google's "secret" for an installed app is public by design,
  and Microsoft public clients have none).
- Boba asks for the **least access** it needs:

| Provider | Send-only | Read and send |
|---|---|---|
| Microsoft Graph | `Mail.Send`, `offline_access` | `Mail.Read`, `Mail.Send`, `offline_access` |
| Gmail | `gmail.send` | `gmail.send`, `gmail.readonly` |

No admin consent and no access without you. Reading is always opt-in, and Boba only reads
messages that belong to your applications, locally.

## Microsoft (Outlook / Microsoft 365)

1. Open the **Microsoft Entra admin center** (<https://entra.microsoft.com>) and sign in.
2. Go to **Applications → App registrations → New registration**.
3. Give it a name (for example `Boba Job Find`), choose **Accounts in any organizational
   directory and personal Microsoft accounts**, and add a redirect URI:
   - Platform: **Mobile and desktop applications** (a public client; no secret is issued).
   - Redirect URI: `http://127.0.0.1` — Microsoft treats loopback redirects from public
     clients as matching any port, which is exactly what Boba needs.
   - If Microsoft later reports a redirect-URI mismatch, add `http://127.0.0.1/cb` to the
     same platform, and use the exact address the browser shows when Boba opens it.
4. Copy the **Application (client) ID** from the app's overview page.
5. In Boba: **Connections → Email → Outlook / Microsoft 365**, paste it into **Your own
   client ID**, choose **Send only** or **Read and send**, and press **Connect Outlook**.
6. Sign in in the browser window Boba opens. Nothing is saved until Microsoft accepts the
   consent.

## Google (Gmail)

1. Open the **Google Cloud console** (<https://console.cloud.google.com>) and create (or
   reuse) a project.
2. **APIs & Services → Library → Gmail API → Enable**.
3. **APIs & Services → OAuth consent screen**: choose **External**, add yourself as a test
   user, and add the scopes `.../auth/gmail.send` and (only if you want Boba to read
   replies) `.../auth/gmail.readonly`.
4. **APIs & Services → Credentials → Create credentials → OAuth client ID**, application
   type **Desktop app**. Desktop clients accept the loopback redirect automatically, so
   there is no redirect URI to register.
5. Copy the **client ID** and the **client secret** Google prints next to it (that value is
   public for installed apps).
6. In Boba: **Connections → Email → Gmail**, paste both into **Your own client ID** and
   **Your client secret**, choose the mode, and press **Connect Gmail**.

Notes for Google:

- While the consent screen is in **Testing**, Google expires refresh tokens after seven
  days and Boba asks you to reconnect. Publish the app to stop that.
- `gmail.readonly` is a Google **restricted** scope. With your own client the consent,
  quota and review obligation stay on your own project; a bundled client would need a
  CASA security assessment, which is why reading Gmail is only offered through your own.
- Gmail sends only by default. Reading needs **Read and send** *and* your own client.

## Where the values go

| Value | Where in Boba |
|---|---|
| Microsoft Application (client) ID | Connections → Email → Outlook, **Your own client ID** |
| Google client ID | Connections → Email → Gmail, **Your own client ID** |
| Google client secret | Connections → Email → Gmail, **Your client secret** |

The same screen shows exactly which scopes Boba will ask for before you connect. When the
build does carry a bundled client, these fields move under **Advanced** and become
optional; when it does not, they are the main way in.

## Troubleshooting

- **The browser window closed or was blocked.** Boba only accepts the answer on
  `127.0.0.1`, in the window it opened. Try again; do not paste the URL anywhere.
- **"Redirect URI mismatch" from the provider.** Re-check the redirect URI above for
  Microsoft, and that the Google client is type **Desktop app**, not **Web application**.
- **"Invalid client" or "unauthorised_client".** The client ID was copied with spaces or
  from another project. Copy it again; for Google, make sure the Gmail API is enabled.
- **Microsoft refuses personal accounts.** The app registration must allow personal
  Microsoft accounts (step 3), not only your organization's directory.
- **You would rather not create a client.** Use the **IMAP / SMTP** form with an app
  password from your provider. Boba tests both connections before saving anything.
