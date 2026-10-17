import { useMutation, useQuery } from "@tanstack/react-query";
import { commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/**
 * The Audit Log (S9.9, FR-89): one read of the merged timeline. The core owns
 * the ordering and the chain verdict; this layer never re-sorts or re-verifies.
 */
export const auditLogKey = ["audit-log"] as const;

export function useAuditLog() {
  return useQuery({
    queryKey: auditLogKey,
    queryFn: () => commands.auditLog().then(unwrap),
  });
}

/**
 * Export to the path the user picked in the native dialog. The core writes the
 * file (AD-13: the webview never chooses a directory on its own).
 */
export function useAuditLogExport() {
  return useMutation({
    mutationFn: (path: string) => commands.auditLogExportCsv(path).then(unwrap),
  });
}
