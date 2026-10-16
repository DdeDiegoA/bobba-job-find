import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
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
    onSuccess: (account: ConnectionView) => {
      queryClient.setQueryData<ConnectionView[]>(emailAccountsKey, (current = []) => [
        ...current.filter((row) => row.id !== account.id),
        account,
      ]);
    },
  });
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
