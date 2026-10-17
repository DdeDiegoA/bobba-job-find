import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { commands, type ManualView } from "@/lib/ipc/bindings";
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
