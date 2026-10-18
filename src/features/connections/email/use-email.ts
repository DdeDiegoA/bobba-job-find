import { type QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  type ConnectGmailInput,
  type ConnectImapSmtpInput,
  type ConnectionView,
  type ConnectOutlookInput,
  commands,
  type EmailMode,
} from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/**
 * The mailbox scopes Boba asks for, per mode (AC1). Presentation only: the core
 * decides what it actually requests (`EmailMode::required_scopes`), so a drift
 * here can never widen the consent Boba asks for.
 */
export const SCOPES_BY_MODE: Record<EmailMode, string[]> = {
  send_only: ["Mail.Send", "offline_access"],
  read_send: ["Mail.Read", "Mail.Send", "offline_access"],
};

/**
 * The Gmail scopes per mode (AC1). Send-only asks for `gmail.send` and nothing
 * else: Google's restricted read scope needs a CASA assessment for a bundled
 * client, so `gmail.readonly` is only ever shown — and only ever requested by
 * the core — for a read+send connection on the user's own client (S8.5).
 * Presentation only, like `SCOPES_BY_MODE`: the core decides what it asks for.
 */
export const GMAIL_SCOPES_BY_MODE: Record<EmailMode, string[]> = {
  send_only: ["https://www.googleapis.com/auth/gmail.send"],
  read_send: [
    "https://www.googleapis.com/auth/gmail.send",
    "https://www.googleapis.com/auth/gmail.readonly",
  ],
};

/**
 * The user guide for creating your own Google or Microsoft OAuth client
 * (FX5b, AC5). Opens in the system browser, never in the app's webview.
 */
export const EMAIL_OWN_CLIENT_GUIDE_URL =
  "https://github.com/DdeDiegoA/bobba-job-find/blob/main/docs/user-guide/email-own-client.md";

/** The connections screen is one query: every connected mailbox (AC1). */
export const emailAccountsKey = ["email", "accounts"] as const;

/** Which one-click providers this build carries (FX5b, AC1). */
export const emailBundledClientsKey = ["email", "bundled_clients"] as const;

/**
 * Which providers have a bundled OAuth client in this build (FX5b, AC1).
 *
 * Booleans only: the core never sends the client id or the Gmail secret. While
 * the probe is loading the app behaves as it always did (both buttons), and a
 * provider it says is absent is hidden instead of failing after the click.
 */
export function useBundledEmailClients() {
  return useQuery({
    queryKey: emailBundledClientsKey,
    queryFn: () => commands.emailBundledClients().then(unwrap),
  });
}

/** Connected mailboxes, oldest first. */
export function useEmailAccounts() {
  return useQuery({
    queryKey: emailAccountsKey,
    queryFn: () => commands.emailAccounts().then(unwrap),
  });
}

/** Connect Outlook; the returned connection replaces the cached list (AC1). */
export function useConnectOutlook() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: ConnectOutlookInput) => commands.emailConnectOutlook(input).then(unwrap),
    onSuccess: (account: ConnectionView) => upsertAccount(queryClient, account),
  });
}

/** Connect Gmail, send-only; the returned connection replaces the cached list. */
export function useConnectGmail() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: ConnectGmailInput) => commands.emailConnectGmail(input).then(unwrap),
    onSuccess: (account: ConnectionView) => upsertAccount(queryClient, account),
  });
}

/**
 * Connect an IMAP/SMTP account (AC2). The core tests both connections first and
 * only then saves, so a wrong password fails here without leaving a connection.
 */
export function useConnectImapSmtp() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: ConnectImapSmtpInput) => commands.emailConnectImapSmtp(input).then(unwrap),
    onSuccess: (account: ConnectionView) => upsertAccount(queryClient, account),
  });
}

/** A reconnected mailbox replaces its row instead of appearing twice. */
function upsertAccount(queryClient: QueryClient, account: ConnectionView) {
  queryClient.setQueryData<ConnectionView[]>(emailAccountsKey, (current = []) => [
    ...current.filter((row) => row.id !== account.id),
    account,
  ]);
}

/** Switch a connection's mode (AC3). */
export function useSetEmailMode() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, mode }: { id: string; mode: EmailMode }) =>
      commands.emailSetMode(id, mode).then(unwrap),
    onSuccess: (account: ConnectionView) => {
      queryClient.setQueryData<ConnectionView[]>(emailAccountsKey, (current = []) =>
        current.map((row) => (row.id === account.id ? account : row)),
      );
    },
  });
}

/** Disconnect a mailbox: revoke where possible, then delete (AC4). */
export function useDisconnectEmail() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => commands.emailDisconnect(id).then(unwrap),
    onSuccess: (_removed: boolean, id: string) => {
      queryClient.setQueryData<ConnectionView[]>(emailAccountsKey, (current = []) =>
        current.filter((row) => row.id !== id),
      );
    },
  });
}
