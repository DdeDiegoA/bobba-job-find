import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import type {
  ConnectionView,
  EmailMode,
  MailCredentialKind,
  MailProvider,
  MailSecurity,
} from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { cn } from "@/lib/utils";
import {
  GMAIL_SCOPES_BY_MODE,
  SCOPES_BY_MODE,
  useConnectGmail,
  useConnectImapSmtp,
  useConnectOutlook,
  useDisconnectEmail,
  useEmailAccounts,
  useSetEmailMode,
} from "./use-email";

const MODES: EmailMode[] = ["send_only", "read_send"];

const SECURITIES: MailSecurity[] = ["tls", "start_tls"];
const CREDENTIAL_KINDS: MailCredentialKind[] = ["password", "xoauth2"];

/** Wording for the two choices, as resolvable keys (AD-21). */
const SECURITY_LABELS: Record<MailSecurity, string> = {
  tls: "connections_email:imap.security.tls",
  start_tls: "connections_email:imap.security.start_tls",
};

const CREDENTIAL_LABELS: Record<MailCredentialKind, string> = {
  password: "connections_email:imap.credential_kind.password",
  xoauth2: "connections_email:imap.credential_kind.xoauth2",
};

/**
 * The provider ports that belong to each encryption choice. A password or token
 * is never sent on an unencrypted socket, so these are the only two options.
 */
const DEFAULT_PORTS: Record<MailSecurity, { imap: number; smtp: number }> = {
  tls: { imap: 993, smtp: 465 },
  start_tls: { imap: 143, smtp: 587 },
};

/**
 * The screen's wording, as qualified `namespace:key` literals (AD-21). Keys live
 * in data so a rename shows up as a missing translation, never as a raw key.
 */
const MODE_LABELS: Record<EmailMode, { label: string; hint: string }> = {
  send_only: {
    label: "connections_email:mode.send_only",
    hint: "connections_email:mode.send_only.hint",
  },
  read_send: {
    label: "connections_email:mode.read_send",
    hint: "connections_email:mode.read_send.hint",
  },
};

/** Plain-language wording for the scopes Boba asks for (AC1). */
const SCOPE_LABELS: Record<string, string> = {
  "Mail.Read": "connections_email:scopes.Mail.Read",
  "Mail.Send": "connections_email:scopes.Mail.Send",
  offline_access: "connections_email:scopes.offline_access",
  "https://www.googleapis.com/auth/gmail.send": "connections_email:scopes.gmail.send",
  "https://www.googleapis.com/auth/gmail.readonly": "connections_email:scopes.gmail.readonly",
};

const PROVIDER_LABELS: Record<MailProvider, string> = {
  microsoft_graph: "connections_email:provider.microsoft_graph",
  gmail: "connections_email:provider.gmail",
  imap_smtp: "connections_email:provider.imap_smtp",
};

/** The plain-language list of what Boba asks the provider for (AC1). */
function ScopeList({ scopes }: { scopes: string[] }) {
  const { t } = useTranslation("connections_email");
  return (
    <ul className="flex flex-col gap-1 text-sm text-fg-muted">
      {scopes.map((scope) => (
        <li key={scope}>{t(SCOPE_LABELS[scope] ?? scope)}</li>
      ))}
    </ul>
  );
}

/** One connected mailbox: its mode switch and its disconnect action (AC3/AC4). */
function ConnectedMailbox({ account }: { account: ConnectionView }) {
  const { t } = useTranslation("connections_email");
  const { t: tError } = useTranslation();
  const setMode = useSetEmailMode();
  const disconnect = useDisconnectEmail();
  const [confirming, setConfirming] = useState(false);

  const modeError =
    setMode.error && isAppError(setMode.error) ? renderAppError(setMode.error, tError) : null;

  return (
    <li className="flex flex-col gap-3 rounded-2xl border border-border bg-surface p-4">
      <div className="flex items-baseline justify-between gap-2">
        <h3 className="font-display text-lg text-fg">{account.address}</h3>
        <span className="flex items-center gap-2">
          {account.client_id_kind === "byo" ? (
            <span className="text-xs text-fg-subtle">{t("list.own_client")}</span>
          ) : null}
          <span className="rounded-xl bg-surface-muted px-2 py-1 text-xs text-fg-muted">
            {t(PROVIDER_LABELS[account.provider])}
          </span>
        </span>
      </div>

      {/* An IMAP/SMTP account has no OAuth grant to switch, so there is no mode
          control: it reads read-only and it sends (AC2). */}
      {account.provider === "imap_smtp" ? (
        <p className="text-sm text-fg-muted">{t("list.imap_smtp")}</p>
      ) : (
        <>
          <fieldset className="flex flex-col gap-2">
            <legend className="text-sm font-medium text-fg">{t("mode")}</legend>
            {MODES.map((mode) => (
              <label key={mode} className="flex items-start gap-2 text-sm">
                <input
                  type="radio"
                  name={`mode-${account.id}`}
                  value={mode}
                  checked={account.mode === mode}
                  disabled={setMode.isPending}
                  onChange={() => setMode.mutate({ id: account.id, mode })}
                />
                <span>
                  <span className="text-fg">{t(MODE_LABELS[mode].label)}</span>
                  <span className="block text-fg-subtle">{t(MODE_LABELS[mode].hint)}</span>
                </span>
              </label>
            ))}
          </fieldset>

          {modeError ? (
            <p role="alert" className="text-sm text-danger">
              {modeError}
            </p>
          ) : null}

          <div className="flex flex-col gap-1">
            <span className="text-sm text-fg-muted">{t("list.scopes")}</span>
            {/* What the provider actually granted, in plain language (AC1). */}
            <ScopeList scopes={account.scopes} />
          </div>
        </>
      )}

      {confirming ? (
        <div className="flex flex-wrap items-center gap-2">
          <p className="text-sm text-fg-muted">
            {t("list.disconnect.confirm", { address: account.address })}
          </p>
          <Button
            variant="outline"
            size="sm"
            disabled={disconnect.isPending}
            onClick={() => {
              disconnect.mutate(account.id);
              setConfirming(false);
            }}
          >
            {t("list.disconnect")}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setConfirming(false)}>
            {t("cancel")}
          </Button>
        </div>
      ) : (
        <div>
          <Button variant="outline" size="sm" onClick={() => setConfirming(true)}>
            {t("list.disconnect")}
          </Button>
        </div>
      )}
    </li>
  );
}

/** The connect form: address, mode, the scopes in plain language, BYO client (AC1). */
function ConnectOutlook() {
  const { t } = useTranslation("connections_email");
  const { t: tError } = useTranslation();
  const connect = useConnectOutlook();
  const [address, setAddress] = useState("");
  const [mode, setMode] = useState<EmailMode>("send_only");
  const [clientId, setClientId] = useState("");

  const error = connect.error && isAppError(connect.error) ? connect.error : null;

  return (
    <form
      className="flex flex-col gap-3 rounded-2xl border border-border bg-surface p-4"
      aria-labelledby="email-connect-title"
      onSubmit={(event) => {
        event.preventDefault();
        connect.mutate({
          address: address.trim(),
          mode,
          client_id: clientId.trim() === "" ? null : clientId.trim(),
        });
      }}
    >
      <h2 id="email-connect-title" className="font-display text-lg text-fg">
        {t("outlook.title")}
      </h2>
      <p className="text-sm text-fg-muted">{t("outlook.description")}</p>

      <label className="flex flex-col gap-1 text-sm text-fg">
        {t("address")}
        <input
          type="email"
          required
          value={address}
          placeholder={t("address.placeholder")}
          onChange={(event) => setAddress(event.target.value)}
          className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
        />
      </label>

      <fieldset className="flex flex-col gap-2">
        <legend className="text-sm font-medium text-fg">{t("mode")}</legend>
        {MODES.map((option) => (
          <label key={option} className="flex items-start gap-2 text-sm">
            <input
              type="radio"
              name="connect-mode"
              value={option}
              checked={mode === option}
              onChange={() => setMode(option)}
            />
            <span>
              <span className="text-fg">{t(MODE_LABELS[option].label)}</span>
              <span className="block text-fg-subtle">{t(MODE_LABELS[option].hint)}</span>
            </span>
          </label>
        ))}
      </fieldset>

      <div className="rounded-xl bg-surface-muted p-3">
        <p className="text-sm font-medium text-fg">{t("scopes.heading")}</p>
        <ScopeList scopes={SCOPES_BY_MODE[mode]} />
        <p className="mt-2 text-xs text-fg-subtle">{t("scopes.least")}</p>
      </div>

      <label className="flex flex-col gap-1 text-sm text-fg">
        {t("client_id")}
        <input
          value={clientId}
          placeholder={t("client_id.placeholder")}
          onChange={(event) => setClientId(event.target.value)}
          className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
        />
        <span className="text-xs text-fg-subtle">{t("client_id.hint")}</span>
      </label>

      <div className="flex items-center gap-2">
        <Button type="submit" disabled={connect.isPending}>
          {connect.isPending ? t("connecting") : t("connect")}
        </Button>
      </div>

      {error ? (
        <div role="alert" className="flex flex-col gap-1 text-sm text-danger">
          <p>{renderAppError(error, tError)}</p>
          {/* AC2: no deep-link fallback — a failed flow points somewhere. */}
          <p className="text-fg-muted">{t("troubleshoot")}</p>
        </div>
      ) : null}
    </form>
  );
}

/**
 * The Gmail connect form (S8.2/S8.5, AC1): address, mode, the scopes, and the
 * advanced client id and public client secret.
 *
 * Reading is opt-in and needs the user's own Google client: Boba's bundled
 * client stays send-only until CASA, so the read option only becomes reachable
 * once a client id is typed. The core enforces the same rule — the form never
 * asks for a consent the core would refuse.
 */
function ConnectGmail() {
  const { t } = useTranslation("connections_email");
  const { t: tError } = useTranslation();
  const connect = useConnectGmail();
  const [address, setAddress] = useState("");
  const [mode, setMode] = useState<EmailMode>("send_only");
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");

  const error = connect.error && isAppError(connect.error) ? connect.error : null;

  // AC1: read+send is the user's own client or nothing.
  const readBlocked = mode === "read_send" && clientId.trim() === "";
  const shownMode = readBlocked ? "send_only" : mode;

  return (
    <form
      className="flex flex-col gap-3 rounded-2xl border border-border bg-surface p-4"
      aria-labelledby="email-connect-gmail-title"
      onSubmit={(event) => {
        event.preventDefault();
        if (readBlocked) return;
        connect.mutate({
          address: address.trim(),
          mode,
          client_id: clientId.trim() === "" ? null : clientId.trim(),
          client_secret: clientSecret.trim() === "" ? null : clientSecret.trim(),
        });
      }}
    >
      <h2 id="email-connect-gmail-title" className="font-display text-lg text-fg">
        {t("gmail.title")}
      </h2>
      <p className="text-sm text-fg-muted">{t("gmail.description")}</p>

      <label className="flex flex-col gap-1 text-sm text-fg">
        {t("address")}
        <input
          type="email"
          required
          value={address}
          placeholder={t("gmail.address.placeholder")}
          onChange={(event) => setAddress(event.target.value)}
          className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
        />
      </label>

      <fieldset className="flex flex-col gap-2">
        <legend className="text-sm font-medium text-fg">{t("mode")}</legend>
        {MODES.map((option) => (
          <label key={option} className="flex items-start gap-2 text-sm">
            <input
              type="radio"
              name="gmail-connect-mode"
              value={option}
              checked={mode === option}
              onChange={() => setMode(option)}
            />
            <span>
              <span className="text-fg">{t(MODE_LABELS[option].label)}</span>
              <span className="block text-fg-subtle">{t(MODE_LABELS[option].hint)}</span>
            </span>
          </label>
        ))}
      </fieldset>

      {readBlocked ? (
        <p role="alert" className="text-xs text-danger">
          {t("gmail.read_needs_client")}
        </p>
      ) : null}

      <div className="rounded-xl bg-surface-muted p-3">
        <p className="text-sm font-medium text-fg">{t("gmail.scopes.heading")}</p>
        <ScopeList scopes={GMAIL_SCOPES_BY_MODE[shownMode]} />
        <p className="mt-2 text-xs text-fg-subtle">
          {shownMode === "send_only" ? t("gmail.send_only") : t("gmail.read_local")}
        </p>
        {shownMode === "read_send" ? (
          <p className="mt-1 text-xs text-fg-subtle">{t("gmail.read_testing")}</p>
        ) : null}
      </div>

      <label className="flex flex-col gap-1 text-sm text-fg">
        {t("client_id")}
        <input
          value={clientId}
          placeholder={t("client_id.placeholder")}
          onChange={(event) => setClientId(event.target.value)}
          className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
        />
        <span className="text-xs text-fg-subtle">{t("client_id.hint")}</span>
      </label>

      <label className="flex flex-col gap-1 text-sm text-fg">
        {t("client_secret")}
        <input
          value={clientSecret}
          placeholder={t("client_secret.placeholder")}
          onChange={(event) => setClientSecret(event.target.value)}
          className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
        />
        <span className="text-xs text-fg-subtle">{t("client_secret.hint")}</span>
      </label>

      <div className="flex items-center gap-2">
        <Button type="submit" disabled={connect.isPending || readBlocked}>
          {connect.isPending ? t("connecting") : t("gmail.connect")}
        </Button>
      </div>

      {error ? (
        <div role="alert" className="flex flex-col gap-1 text-sm text-danger">
          <p>{renderAppError(error, tError)}</p>
          <p className="text-fg-muted">{t("troubleshoot")}</p>
        </div>
      ) : null}
    </form>
  );
}

/**
 * The IMAP/SMTP connect form (S8.2, AC2): the endpoints of both protocols, one
 * credential, and the encryption for each. "Test and save" is honest naming: the
 * core probes IMAP (read-only) and the submission endpoint first, and saves the
 * connection only when both answer.
 */
function ConnectImapSmtp() {
  const { t } = useTranslation("connections_email");
  const { t: tError } = useTranslation();
  const connect = useConnectImapSmtp();
  const [address, setAddress] = useState("");
  const [username, setUsername] = useState("");
  const [credential, setCredential] = useState("");
  const [credentialKind, setCredentialKind] = useState<MailCredentialKind>("password");
  const [imapHost, setImapHost] = useState("");
  const [imapPort, setImapPort] = useState(DEFAULT_PORTS.tls.imap);
  const [imapSecurity, setImapSecurity] = useState<MailSecurity>("tls");
  const [smtpHost, setSmtpHost] = useState("");
  const [smtpPort, setSmtpPort] = useState(DEFAULT_PORTS.tls.smtp);
  const [smtpSecurity, setSmtpSecurity] = useState<MailSecurity>("tls");

  const error = connect.error && isAppError(connect.error) ? connect.error : null;

  return (
    <form
      className="flex flex-col gap-3 rounded-2xl border border-border bg-surface p-4"
      aria-labelledby="email-connect-imap-title"
      onSubmit={(event) => {
        event.preventDefault();
        connect.mutate({
          address: address.trim(),
          username: username.trim() === "" ? address.trim() : username.trim(),
          credential,
          credential_kind: credentialKind,
          imap_host: imapHost.trim(),
          imap_port: imapPort,
          imap_security: imapSecurity,
          smtp_host: smtpHost.trim(),
          smtp_port: smtpPort,
          smtp_security: smtpSecurity,
        });
      }}
    >
      <h2 id="email-connect-imap-title" className="font-display text-lg text-fg">
        {t("imap.title")}
      </h2>
      <p className="text-sm text-fg-muted">{t("imap.description")}</p>

      <label className="flex flex-col gap-1 text-sm text-fg">
        {t("address")}
        <input
          type="email"
          required
          value={address}
          placeholder={t("address.placeholder")}
          onChange={(event) => setAddress(event.target.value)}
          className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
        />
      </label>

      <label className="flex flex-col gap-1 text-sm text-fg">
        {t("imap.username")}
        <input
          value={username}
          placeholder={t("address.placeholder")}
          onChange={(event) => setUsername(event.target.value)}
          className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
        />
        <span className="text-xs text-fg-subtle">{t("imap.username.hint")}</span>
      </label>

      <label className="flex flex-col gap-1 text-sm text-fg">
        {t("imap.credential_kind")}
        <select
          value={credentialKind}
          onChange={(event) => setCredentialKind(event.target.value as MailCredentialKind)}
          className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
        >
          {CREDENTIAL_KINDS.map((kind) => (
            <option key={kind} value={kind}>
              {t(CREDENTIAL_LABELS[kind])}
            </option>
          ))}
        </select>
      </label>

      <label className="flex flex-col gap-1 text-sm text-fg">
        {t("imap.credential")}
        <input
          type="password"
          required
          value={credential}
          autoComplete="off"
          onChange={(event) => setCredential(event.target.value)}
          className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
        />
      </label>

      <fieldset className="grid grid-cols-2 gap-2">
        <legend className="text-sm font-medium text-fg">{t("imap.host")}</legend>
        <input
          required
          value={imapHost}
          aria-label={t("imap.host")}
          placeholder="imap.example.com"
          onChange={(event) => setImapHost(event.target.value)}
          className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
        />
        <input
          type="number"
          required
          min={1}
          max={65535}
          value={imapPort}
          aria-label={t("imap.port")}
          onChange={(event) => setImapPort(Number(event.target.value))}
          className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
        />
        {/* The port follows the encryption choice until the user types their own. */}
        <select
          value={imapSecurity}
          aria-label={t("imap.security")}
          onChange={(event) => {
            const next = event.target.value as MailSecurity;
            if (imapPort === DEFAULT_PORTS[imapSecurity].imap)
              setImapPort(DEFAULT_PORTS[next].imap);
            setImapSecurity(next);
          }}
          className="col-span-2 h-10 rounded-xl border border-border bg-surface px-3 text-fg"
        >
          {SECURITIES.map((security) => (
            <option key={security} value={security}>
              {t(SECURITY_LABELS[security])}
            </option>
          ))}
        </select>
      </fieldset>

      <fieldset className="grid grid-cols-2 gap-2">
        <legend className="text-sm font-medium text-fg">{t("imap.smtp_host")}</legend>
        <input
          required
          value={smtpHost}
          aria-label={t("imap.smtp_host")}
          placeholder="smtp.example.com"
          onChange={(event) => setSmtpHost(event.target.value)}
          className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
        />
        <input
          type="number"
          required
          min={1}
          max={65535}
          value={smtpPort}
          aria-label={t("imap.smtp_port")}
          onChange={(event) => setSmtpPort(Number(event.target.value))}
          className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
        />
        <select
          value={smtpSecurity}
          aria-label={t("imap.smtp_security")}
          onChange={(event) => {
            const next = event.target.value as MailSecurity;
            if (smtpPort === DEFAULT_PORTS[smtpSecurity].smtp)
              setSmtpPort(DEFAULT_PORTS[next].smtp);
            setSmtpSecurity(next);
          }}
          className="col-span-2 h-10 rounded-xl border border-border bg-surface px-3 text-fg"
        >
          {SECURITIES.map((security) => (
            <option key={security} value={security}>
              {t(SECURITY_LABELS[security])}
            </option>
          ))}
        </select>
      </fieldset>

      <p className="text-xs text-fg-subtle">{t("imap.read_only")}</p>

      <div className="flex items-center gap-2">
        <Button type="submit" disabled={connect.isPending}>
          {connect.isPending ? t("imap.testing") : t("imap.save")}
        </Button>
      </div>

      {error ? (
        <div role="alert" className="flex flex-col gap-1 text-sm text-danger">
          <p>{renderAppError(error, tError)}</p>
          <p className="text-fg-muted">{t("troubleshoot")}</p>
        </div>
      ) : null}
    </form>
  );
}

/**
 * Email connections (S8.1, S8.2, EXPERIENCE §5). Connects Outlook or Gmail
 * through the loopback OAuth flow, switches a connection between send-only and
 * read+send, and disconnects it.
 */
export default function ConnectionsEmail() {
  const { t } = useTranslation("connections_email");
  const accounts = useEmailAccounts();
  const rows = accounts.data ?? [];

  return (
    <section className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-6">
      <div className="flex flex-col gap-1">
        <h1 className="font-display text-2xl text-fg">{t("title")}</h1>
        <p className="text-sm text-fg-muted">{t("subtitle")}</p>
      </div>

      {accounts.isLoading ? <p className="text-sm text-fg-muted">{t("loading")}</p> : null}
      {accounts.isError ? (
        <p role="alert" className="text-sm text-danger">
          {t("error.load")}
        </p>
      ) : null}

      {!accounts.isLoading && rows.length === 0 ? (
        <EmptyState title={t("empty")} description={t("empty.hint")} />
      ) : null}

      {rows.length > 0 ? (
        <ul className="flex flex-col gap-3">
          {rows.map((account) => (
            <ConnectedMailbox key={account.id} account={account} />
          ))}
        </ul>
      ) : null}

      <div className={cn("flex flex-col gap-3", rows.length > 0 && "border-border border-t pt-4")}>
        <ConnectOutlook />
        <ConnectGmail />
        <ConnectImapSmtp />
      </div>
    </section>
  );
}
