import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type ApplicationDetail, commands, type TransitionInput } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/** The board is one query: every column with its cards (AC1). */
export const boardKey = ["tracker", "board"] as const;

/** The card detail is per application and only fetched while it is open (AC3). */
export const detailKey = (applicationId: string) => ["tracker", "detail", applicationId] as const;

/** Every Stage column with its cards. */
export function useBoard() {
  return useQuery({
    queryKey: boardKey,
    queryFn: () => commands.trackerBoard().then(unwrap),
  });
}

/** One card in full: snapshot, package, timeline, next action, notes. */
export function useApplicationDetail(applicationId: string | null) {
  return useQuery({
    queryKey: detailKey(applicationId ?? ""),
    enabled: applicationId !== null,
    queryFn: () => commands.trackerApplicationDetail(applicationId as string).then(unwrap),
  });
}

/** Move one card; the board is re-read because the move may have lost a race. */
export function useTransitionStatus() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: TransitionInput) => commands.trackerTransitionStatus(input).then(unwrap),
    onSuccess: (detail: ApplicationDetail) => {
      queryClient.setQueryData(detailKey(detail.id), detail);
      void queryClient.invalidateQueries({ queryKey: boardKey });
      void queryClient.invalidateQueries({ queryKey: detailKey(detail.id) });
    },
  });
}
