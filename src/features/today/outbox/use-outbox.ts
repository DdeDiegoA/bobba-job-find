import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/** The Outbox panel is one read (S9.7, FR-84/FR-85). */
export const outboxKey = ["apply", "outbox"] as const;

/** The queued items and every rule's dry run. */
export function useOutbox() {
  return useQuery({
    queryKey: outboxKey,
    queryFn: () => commands.applyOutboxView().then(unwrap),
  });
}

/** The hold every automatic send waits (FR-85); core validates the range. */
export function useSetHold() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (minutes: number) => commands.applyOutboxSetHold(minutes).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: outboxKey }),
  });
}

/** Cancel one item during its hold. */
export function useCancelItem() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => commands.applyOutboxCancel(id).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: outboxKey }),
  });
}

/** Edit the subject or the body before the send leaves (FR-85). */
export function useEditItem() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      id,
      subject,
      body,
    }: {
      id: string;
      subject: string | null;
      body: string | null;
    }) => commands.applyOutboxEdit(id, subject, body).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: outboxKey }),
  });
}

/** End a rule's dry run and let it send from now on (FR-84). */
export function useActivateRule() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (ruleId: string) => commands.applyOutboxActivateRule(ruleId).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: outboxKey }),
  });
}

/** How many matches a rule's dry run reports (FR-84, minimum 1). */
export function useSetDryRun() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ ruleId, matches }: { ruleId: string; matches: number }) =>
      commands.applyOutboxSetDryRun(ruleId, matches).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: outboxKey }),
  });
}
