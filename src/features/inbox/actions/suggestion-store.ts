import { create } from "zustand";
import type { SuggestionView } from "@/lib/ipc/bindings";

/**
 * The Preferences change Boba is currently suggesting (S6.6, FR-67 AC3).
 *
 * It arrives with the third dismissal (the core decides), so it is kept here —
 * one small store — rather than in a card that is about to leave the list. The
 * banner reads it; accepting or ignoring clears it.
 */
interface SuggestionState {
  /** The change on offer, `null` when there is none. */
  current: SuggestionView | null;
  /** Show the change the core suggested. */
  suggest: (suggestion: SuggestionView) => void;
  /** Drop it (answered, or the banner was dismissed). */
  clear: () => void;
}

export const useSuggestionStore = create<SuggestionState>((set) => ({
  current: null,
  suggest: (suggestion) => set({ current: suggestion }),
  clear: () => set({ current: null }),
}));
