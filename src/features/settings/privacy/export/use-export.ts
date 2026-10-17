import { useMutation } from "@tanstack/react-query";
import { commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/**
 * The export surface (S10.1, FR-110): one mutation, because the core owns
 * every format and every file. The page only carries the folder the user
 * picked in a native dialog, after the unencrypted warning (AC1).
 */
export function useExportData() {
  return useMutation({
    mutationFn: (destination: string) => commands.exportData(destination).then(unwrap),
  });
}
