import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/** The pending Status Proposals and the reply-reading settings (S8.4, AC2/AC3). */
export const proposalsKey = ["status_proposals", "pending"] as const;
export const disclosureKey = ["status_proposals", "disclosure"] as const;

/** The proposals waiting in Today's needs-you queue. */
export function useProposals() {
  return useQuery({
    queryKey: proposalsKey,
    queryFn: () => commands.statusProposalsPending().then(unwrap),
  });
}

/** The disclosure and the modes the user chose (AC3). */
export function useDisclosure() {
  return useQuery({
    queryKey: disclosureKey,
    queryFn: () => commands.statusProposalsDisclosure().then(unwrap),
  });
}

/** Accept the disclosure, with the classifier the user chose (AC3). */
export function useSaveDisclosure() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { mode: string; autoConfirm: boolean; acknowledge: boolean }) =>
      commands
        .statusProposalsSaveSettings(input.mode, input.autoConfirm, input.acknowledge)
        .then(unwrap),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: disclosureKey });
      void queryClient.invalidateQueries({ queryKey: proposalsKey });
    },
  });
}

/** Read every message no classifier has read yet (AC1, AC4). */
export function useClassify() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => commands.statusProposalsClassify().then(unwrap),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: proposalsKey });
      void queryClient.invalidateQueries({ queryKey: disclosureKey });
    },
  });
}

/** Approve one proposal, moving its card (AC2). */
export function useApprove() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { id: string; applicationId: string | null }) =>
      commands.statusProposalsApprove(input.id, input.applicationId).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: proposalsKey }),
  });
}

/** Approve several at once; the ambiguous ones come back reported (AC2). */
export function useApproveMany() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (ids: string[]) => commands.statusProposalsApproveMany(ids).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: proposalsKey }),
  });
}

/** Dismiss a proposal: nothing moves (AC2). */
export function useDismiss() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => commands.statusProposalsDismiss(id).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: proposalsKey }),
  });
}

/** Undo an automatically applied change (AC5). */
export function useUndo() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => commands.statusProposalsUndo(id).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: proposalsKey }),
  });
}
