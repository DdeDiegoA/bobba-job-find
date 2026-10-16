import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { commands, type Preferences, type PreferencesSnapshot } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/**
 * The preferences document as the core holds it (S3.7): one query, because the
 * step and the Settings page show the same thing. The core owns the version and
 * the stale flag (FR-14); this layer never invents either.
 */
export const preferencesKey = ["preferences"] as const;

/** The stored preferences, their version and whether the scores are stale. */
export function usePreferences() {
  return useQuery({
    queryKey: preferencesKey,
    queryFn: () => commands.preferencesSnapshot().then(unwrap),
  });
}

/** Save an edit; the core appends the version and returns the new document. */
export function useSavePreferences() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (values: Preferences) => commands.preferencesSave(values).then(unwrap),
    onSuccess: (snapshot: PreferencesSnapshot) =>
      queryClient.setQueryData(preferencesKey, snapshot),
  });
}
