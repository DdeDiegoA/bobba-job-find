import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/** Every Session Board Source and where its Auto stands (S11.3, FR-91). */
export const sessionAutoKey = ["apply", "session-auto"] as const;

export function useSessionAuto() {
  return useQuery({
    queryKey: sessionAutoKey,
    queryFn: () => commands.sessionAutoSnapshot().then(unwrap),
  });
}

/** Accept the separate Auto opt-in: the core compares the typed name. */
export function useSessionAutoOptIn() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      sourceId,
      typedName,
      warningVersion,
    }: {
      sourceId: string;
      typedName: string;
      warningVersion: string;
    }) => commands.sessionAutoOptIn(sourceId, typedName, warningVersion).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: sessionAutoKey }),
  });
}

/** Revoke Auto; the Session Board reading opt-in stays. */
export function useSessionAutoRevoke() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (sourceId: string) => commands.sessionAutoRevoke(sourceId).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: sessionAutoKey }),
  });
}

/** Set the per-Source cap; the core rejects anything above ten a day. */
export function useSessionAutoCap() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ sourceId, cap }: { sourceId: string; cap: number }) =>
      commands.sessionAutoSetCap(sourceId, cap).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: sessionAutoKey }),
  });
}
