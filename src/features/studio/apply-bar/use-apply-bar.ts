import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { commands, type ManualView, type SendRequest } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/** One application's channels and kit (S9.2): one read, replaced by every write. */
export const applyBarKey = (applicationId: string) => ["apply", applicationId] as const;

export function useApplyView(applicationId: string) {
  return useQuery({
    queryKey: applyBarKey(applicationId),
    enabled: applicationId.length > 0,
    queryFn: () => commands.applyView(applicationId).then(unwrap),
  });
}

/**
 * Mark the application Applied after the user applied by hand. The core
 * re-checks Ready + verified and answers with the whole view (AD-14).
 */
export function useMarkApplied(applicationId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => commands.applyMarkApplied(applicationId).then(unwrap),
    onSuccess: (view: ManualView) => queryClient.setQueryData(applyBarKey(applicationId), view),
  });
}

/** Write the kit's verified PDFs to disk, returning where they went. */
export function useExportPdfs(applicationId: string) {
  return useMutation({
    mutationFn: () => commands.applyExportPdfs(applicationId).then(unwrap),
  });
}

/** The reviewed email draft, its verified attachments and the evidence (S9.3, AC1). */
export const emailReviewKey = (applicationId: string) => ["apply", applicationId, "email"] as const;

export function useEmailReview(applicationId: string) {
  return useQuery({
    queryKey: emailReviewKey(applicationId),
    enabled: applicationId.length > 0,
    queryFn: () => commands.applyEmailReview(applicationId).then(unwrap),
  });
}

/**
 * Send the reviewed application (S9.3, AC1). The core decides — evidence,
 * AD-14 gate, audit — and the whole bar is re-read afterwards, because the card
 * moved to Applied.
 */
export function useEmailSend(applicationId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: SendRequest) =>
      commands.applyEmailSend(applicationId, request).then(unwrap),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: emailReviewKey(applicationId) });
      void queryClient.invalidateQueries({ queryKey: applyBarKey(applicationId) });
    },
  });
}
