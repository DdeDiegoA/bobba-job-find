import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/**
 * The backup surface (S10.2, FR-111/FR-112): one query for the recovery-backup
 * state, one mutation per action. Every decision lives in the core; the page
 * only carries the path the user picked in a native dialog.
 */
export const backupStatusKey = ["backup", "status"] as const;

/** Whether a recovery backup exists and where the last one went (AC1/AC2). */
export function useBackupStatus() {
  return useQuery({
    queryKey: backupStatusKey,
    queryFn: () => commands.backupStatus().then(unwrap),
  });
}

/** Write one passphrase-encrypted backup (AC1). */
export function useCreateBackup() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { destination: string; passphrase: string }) =>
      commands.backupCreate(input.destination, input.passphrase).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: backupStatusKey }),
  });
}

/** Replace the current data with a backup (AC3/AC4). */
export function useRestoreBackup() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { source: string; passphrase: string }) =>
      commands.backupRestore(input.source, input.passphrase).then(unwrap),
    // Every screen reads data that may have just been replaced.
    onSuccess: () => queryClient.invalidateQueries(),
  });
}

/** Delete everything after the typed confirmation (AC5). */
export function useResetEverything() {
  return useMutation({
    mutationFn: (confirmation: string) => commands.resetEverything(confirmation).then(unwrap),
  });
}
