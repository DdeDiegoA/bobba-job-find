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
 * The one scope the Gmail connection asks for (AC1): `gmail.send`. Sending only,
 * by design — Google's restricted read scope needs a CASA assessment, so reading
 * Gmail goes through IMAP (S8.3), a BYO client (S8.5) or a `.eml` import.
 */
export const GMAIL_SCOPES = ["https://www.googleapis.com/auth/gmail.send"];

/** The connections screen is one query: every connected mailbox (AC1). */
export const emailAccountsKey = ["email", "accounts"] as const;

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
