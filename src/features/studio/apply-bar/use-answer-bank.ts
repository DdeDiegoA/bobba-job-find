import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type AnswerBankView, commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/** The bank is one query per application: drafts and approved rows (S7.8). */
export const answerBankKey = (applicationId: string) =>
  ["apply", "answers", applicationId] as const;

export function useAnswers(applicationId: string) {
  return useQuery({
    queryKey: answerBankKey(applicationId),
    enabled: applicationId.length > 0,
    queryFn: () => commands.applyAnswers(applicationId).then(unwrap),
  });
}

/**
 * Approve one row, edited or as it stands (S7.8 AC3). The core answers with
 * the whole bank, which replaces the cached one.
 */
export function useApproveAnswer(applicationId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ answerId, answer }: { answerId: string; answer: string | null }) =>
      commands.applyAnswerApprove(applicationId, answerId, answer).then(unwrap),
    onSuccess: (bank: AnswerBankView) =>
      queryClient.setQueryData(answerBankKey(applicationId), bank),
  });
}
