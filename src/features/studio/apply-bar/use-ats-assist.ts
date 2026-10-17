import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/** Whether the user accepted the browser warning (S9.4 AC1). */
export const atsOptInKey = ["apply", "ats", "opt-in"] as const;

export function useAtsOptIn() {
  return useQuery({
    queryKey: atsOptInKey,
    queryFn: () => commands.applyAtsOptedIn().then(unwrap),
  });
}

/** Turn form filling on or off; the core records the consent in the audit log. */
export function useSetAtsOptIn() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (enabled: boolean) => commands.applyAtsOptIn(enabled).then(unwrap),
    onSuccess: (enabled: boolean) => queryClient.setQueryData(atsOptInKey, enabled),
  });
}

/** Open the ATS form in the user's own Chrome and fill it (AC1/AC2). */
export function useAtsStart(applicationId: string) {
  return useMutation({
    mutationFn: () => commands.applyAtsStart(applicationId).then(unwrap),
  });
}

/**
 * Read the page after the user submitted (AC3). The card may have moved, so
 * both the apply bar and the Tracker's card list are re-read.
 */
export function useAtsConfirm(applicationId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (attemptId: string) =>
      commands.applyAtsConfirm(applicationId, attemptId).then(unwrap),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["apply", applicationId] });
      void queryClient.invalidateQueries({ queryKey: ["studio", "applications"] });
    },
  });
}
