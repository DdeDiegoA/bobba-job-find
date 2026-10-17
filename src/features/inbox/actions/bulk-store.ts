import { create } from "zustand";

/**
 * Bulk selection for the Inbox (S6.6, FR-66: "Bulk dismiss works").
 *
 * Selection is a mode, not a filter: it is off until the user turns it on, and
 * turning it off clears it. The cards render their own checkbox from this store
 * and the bar reads the same state, so no list state is duplicated.
 */
interface BulkState {
  /** Whether the checkboxes are showing. */
  active: boolean;
  /** The Offers picked, in the order they were picked. */
  selected: string[];
  /** Show the checkboxes and start a selection. */
  start: () => void;
  /** Leave the mode and clear the selection. */
  stop: () => void;
  /** Pick or unpick one Offer. */
  toggle: (postingId: string) => void;
  /** Pick every id that is not picked yet (Select all shown). */
  selectAll: (postingIds: string[]) => void;
  /** Unpick everything, staying in the mode. */
  clear: () => void;
  /** Drop the ids that just left the list (after a dismissal). */
  forget: (postingIds: string[]) => void;
}

export const useBulkStore = create<BulkState>((set) => ({
  active: false,
  selected: [],
  start: () => set({ active: true, selected: [] }),
  stop: () => set({ active: false, selected: [] }),
  toggle: (postingId) =>
    set((state) => ({
      selected: state.selected.includes(postingId)
        ? state.selected.filter((id) => id !== postingId)
        : [...state.selected, postingId],
    })),
  selectAll: (postingIds) =>
    set((state) => ({
      selected: [...new Set([...state.selected, ...postingIds])],
    })),
  clear: () => set({ selected: [] }),
  forget: (postingIds) =>
    set((state) => ({
      selected: state.selected.filter((id) => !postingIds.includes(id)),
    })),
}));
