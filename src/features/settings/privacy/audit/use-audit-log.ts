import { useMutation, useQuery } from "@tanstack/react-query";
import { commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/**
 * The Audit Log (S9.9, FR-89): one page of the merged timeline at a time.
 * The core owns the ordering, the pagination and the chain verdict; this layer
 * never re-sorts or re-verifies (CH12: the whole log never crosses IPC).
 */
export const auditLogKey = (cursor: string | null) => ["audit-log", cursor] as const;

export function useAuditLog(cursor: string | null) {
  return useQuery({
    queryKey: auditLogKey(cursor),
    queryFn: () => commands.auditLog(cursor ?? null, PAGE_SIZE).then(unwrap),
    // No placeholder from the previous page: showing page 1's rows under a
    // "Page 2" label while page 2 loads would mislabel what the user sees.
  });
}

/** How many rows one page asks the core for (the core clamps it too). */
export const PAGE_SIZE = 50;

/**
 * Export to the path the user picked in the native dialog. The core writes the
 * file (AD-13: the webview never chooses a directory on its own).
 */
export function useAuditLogExport() {
  return useMutation({
    mutationFn: (path: string) => commands.auditLogExportCsv(path).then(unwrap),
  });
}
