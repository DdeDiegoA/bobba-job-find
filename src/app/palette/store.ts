import { create } from "zustand";

/**
 * Whether the command palette is open (S6.7).
 *
 * The palette is mounted once by the shell; the topbar trigger and the global
 * Cmd/Ctrl+K shortcut only flip this store, so neither owns the dialog.
 */
interface PaletteState {
  open: boolean;
  show: () => void;
  hide: () => void;
}

export const usePaletteStore = create<PaletteState>((set) => ({
  open: false,
  show: () => set({ open: true }),
  hide: () => set({ open: false }),
}));

/** Open the palette from anywhere (the topbar's search trigger). */
export function openPalette(): void {
  usePaletteStore.getState().show();
}
