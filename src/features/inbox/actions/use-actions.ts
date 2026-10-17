import { useMutation, useQueryClient } from "@tanstack/react-query";
import { boardKey } from "@/features/tracker/board/use-tracker";
import { commands, type DismissOutcome } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";
import { inboxKey } from "../list/offers";
import { useSuggestionStore } from "./suggestion-store";

/**
 * The Inbox actions' mutations (S6.6, FR-54, FR-66, FR-67).
 *
 * Every action is one core command; nothing is decided here. After a dismissal
 * or an override the Inbox is re-read (a dismissed card leaves the sections, an
 * overridden one moves out of Excluded), and Save also refreshes the Tracker
 * board, where the new card appeared (FR-66).
 */

/** Save an Offer: an Application in Saved (AC1). */
export function useSaveOffer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (postingId: string) => commands.inboxSave(postingId).then(unwrap),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: boardKey });
      void queryClient.invalidateQueries({ queryKey: inboxKey });
    },
  });
}

/**
 * Dismiss one or more Offers with an optional reason (AC1). The suggestion the
 * core hands back when this was the third of a kind (AC3) is kept for the
 * banner, so it appears without a second read.
 */
export function useDismissOffer() {
  const queryClient = useQueryClient();
  const suggest = useSuggestionStore((state) => state.suggest);
  return useMutation({
    mutationFn: ({
      postingIds,
      reason,
    }: {
      postingIds: string[];
      reason: string | null;
    }): Promise<DismissOutcome> => commands.inboxDismiss(postingIds, reason).then(unwrap),
    onSuccess: (outcome) => {
      if (outcome.suggestion) suggest(outcome.suggestion);
      void queryClient.invalidateQueries({ queryKey: inboxKey });
    },
  });
}

/** Undo a Dismiss: the reason and the dismissal are both cleared (AC1). */
export function useUndoDismiss() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (postingId: string) => commands.inboxUndoDismiss(postingId).then(unwrap),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: inboxKey });
    },
  });
}

/** Override the Gate that vetoed an Excluded Offer (AC2). */
export function useOverrideVeto() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (postingId: string) => commands.inboxOverrideVeto(postingId).then(unwrap),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: inboxKey });
    },
  });
}

/** Accept or ignore the Preferences change Boba suggested (AC3). */
export function useDecideSuggestion() {
  const queryClient = useQueryClient();
  const clear = useSuggestionStore((state) => state.clear);
  return useMutation({
    mutationFn: ({
      id,
      accept,
      company,
    }: {
      id: string;
      accept: boolean;
      company?: string | null;
    }) => commands.inboxDecideSuggestion(id, accept, company ?? null).then(unwrap),
    onSuccess: () => {
      clear();
      // Accepting wrote the Preferences (S3.7) and made the scores stale.
      void queryClient.invalidateQueries({ queryKey: inboxKey });
    },
  });
}
