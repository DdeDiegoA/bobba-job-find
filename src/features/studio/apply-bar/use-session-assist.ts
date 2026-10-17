import { useMutation, useQueryClient } from "@tanstack/react-query";
import { commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/** Open the board's apply page in the user's own session and fill it (S11.2). */
export function useSessionStart(applicationId: string) {
  return useMutation({
    mutationFn: () => commands.applySessionStart(applicationId).then(unwrap),
  });
}

/**
 * Fill the step the user advanced to, under the same attempt (LinkedIn Easy
 * Apply moves in steps). Nothing opens: the window from the start is reused.
 */
export function useSessionFill(applicationId: string) {
  return useMutation({
    mutationFn: (attemptId: string) =>
      commands.applySessionFill(applicationId, attemptId).then(unwrap),
  });
}

/**
 * Read the page after the user submitted. The card may have moved, so both the
 * apply bar and the Tracker's card list are re-read.
 */
export function useSessionConfirm(applicationId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (attemptId: string) =>
      commands.applySessionConfirm(applicationId, attemptId).then(unwrap),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["apply", applicationId] });
      void queryClient.invalidateQueries({ queryKey: ["studio", "applications"] });
    },
  });
}
