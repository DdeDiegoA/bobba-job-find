import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/**
 * The Session Board surface (S11.1a, FR-30): one query, because the
 * connections card and the Privacy page must show the same opt-in, the same
 * pause and the same cookie protection the core decided.
 */
export const sessionSnapshotKey = ["session", "snapshot"] as const;

/** Every Session Board Source, the warning version and the cookie protection. */
export function useSessionSnapshot() {
  return useQuery({
    queryKey: sessionSnapshotKey,
    queryFn: () => commands.sessionSnapshot().then(unwrap),
  });
}

/**
 * Record the per-Source opt-in (AC1/AC2). The core refuses a stale warning
 * version and, on Linux without a keyring, a missing weak-cookie ack, so the
 * UI cannot talk its way past either.
 */
export function useSessionOptIn() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { sourceId: string; warningVersion: string; weakCookieAck: boolean }) =>
      commands.sessionOptIn(input.sourceId, input.warningVersion, input.weakCookieAck).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: sessionSnapshotKey }),
  });
}

/** Turn a source off and drop its opt-in; cookies stay until sign-out (AC4). */
export function useSessionOptOut() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (sourceId: string) => commands.sessionOptOut(sourceId).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: sessionSnapshotKey }),
  });
}

/** Delete that source's session and cookies (AC3). */
export function useSessionSignOut() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (sourceId: string) => commands.sessionSignOut(sourceId).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: sessionSnapshotKey }),
  });
}

/** Clear a challenge pause once the user acted on it (FR-30). */
export function useSessionResume() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (sourceId: string) => commands.sessionResume(sourceId).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: sessionSnapshotKey }),
  });
}
