import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { currentLocale } from "@/lib/i18n";
import {
  commands,
  type FactEdit,
  type FactRevisionInput,
  type ProfileSnapshot,
} from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/** The Profile screen is one query: the current facts plus the version (AC1). */
export const profileKey = ["profile"] as const;

/** History is per item and only fetched while its panel is open (AC3). */
export const historyKey = (itemId: string) => ["profile", "history", itemId] as const;

/** The current profile snapshot. */
export function useProfile() {
  return useQuery({
    queryKey: profileKey,
    queryFn: () => commands.profileSnapshot().then(unwrap),
  });
}

/** Save a new fact or an edit; the returned snapshot replaces the cached one. */
export function useSaveFact() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: FactRevisionInput) => commands.profileSaveFact(input).then(unwrap),
    onSuccess: (snapshot: ProfileSnapshot) => queryClient.setQueryData(profileKey, snapshot),
  });
}

/** Every revision of one fact, newest first. */
export function useFactHistory(itemId: string | null) {
  return useQuery({
    queryKey: historyKey(itemId ?? ""),
    enabled: itemId !== null,
    queryFn: () => commands.profileHistory(itemId as string).then(unwrap),
  });
}

/**
 * Draft a fact's missing language (FX8 AC2). The run goes through the same
 * agent runtime and lockdown gate as onboarding's, and it only stores a
 * proposal: nothing reaches the profile until [`useConfirmProposal`] runs.
 */
export function useDraftLanguage() {
  return useMutation({
    mutationFn: (input: { itemId: string; lang: string }) =>
      commands.profileGapsTranslate(input.itemId, input.lang, currentLocale()).then(unwrap),
  });
}

/** Save a reviewed draft as the next profile version (FX8 AC3). */
export function useConfirmProposal() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { id: string; edit: FactEdit | null }) =>
      commands.profileGapsConfirm(input.id, input.edit).then(unwrap),
    onSuccess: (snapshot: ProfileSnapshot) => queryClient.setQueryData(profileKey, snapshot),
  });
}

/** Delete a draft: it never becomes a fact (FX8 AC3). */
export function useDiscardProposal() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => commands.profileGapsDiscard(id).then(unwrap),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: profileKey });
    },
  });
}

/** Restore an earlier revision; it lands as the newest one (AC3). */
export function useRestoreFact(itemId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (revisionId: string) => commands.profileRestore(itemId, revisionId).then(unwrap),
    onSuccess: (snapshot: ProfileSnapshot) => {
      queryClient.setQueryData(profileKey, snapshot);
      void queryClient.invalidateQueries({ queryKey: historyKey(itemId) });
    },
  });
}
