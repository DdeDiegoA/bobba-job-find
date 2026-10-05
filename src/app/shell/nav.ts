import type { LucideIcon } from "lucide-react";
import {
  FileText,
  Home,
  Inbox,
  Plug2,
  SlidersHorizontal,
  SquareKanban,
  UserRound,
} from "lucide-react";

/** A primary-navigation entry (DESIGN.md §10.4 sidebar list). */
export interface NavItem {
  /** Route path (react-router). */
  to: string;
  /** i18n key under the `shell` namespace. */
  labelKey: string;
  icon: LucideIcon;
  /** `Cmd/Ctrl + <digit>` (1-based). */
  digit: number;
  /** `g` then this key (mouseless nav). `,` reaches Settings. */
  goto: string;
}

export const NAV_ITEMS: readonly NavItem[] = [
  { to: "/", labelKey: "shell:nav.today", icon: Home, digit: 1, goto: "t" },
  { to: "/inbox", labelKey: "shell:nav.inbox", icon: Inbox, digit: 2, goto: "i" },
  { to: "/studio", labelKey: "shell:nav.studio", icon: FileText, digit: 3, goto: "s" },
  { to: "/tracker", labelKey: "shell:nav.tracker", icon: SquareKanban, digit: 4, goto: "k" },
  { to: "/profile", labelKey: "shell:nav.profile", icon: UserRound, digit: 5, goto: "p" },
  { to: "/connections", labelKey: "shell:nav.connections", icon: Plug2, digit: 6, goto: "c" },
  { to: "/settings", labelKey: "shell:nav.settings", icon: SlidersHorizontal, digit: 7, goto: "," },
];

/** Topbar title per route prefix (longest match wins). */
const PAGE_TITLES: readonly { prefix: string; labelKey: string }[] = [
  { prefix: "/offer", labelKey: "shell:title.offer" },
  ...NAV_ITEMS.map((item) => ({
    prefix: item.to === "/" ? "/" : item.to,
    labelKey: item.labelKey,
  })),
];

export function pageTitleKey(pathname: string): string {
  const match = PAGE_TITLES.filter((p) => pathname.startsWith(p.prefix)).sort(
    (a, b) => b.prefix.length - a.prefix.length,
  )[0];
  return match?.labelKey ?? "shell:nav.today";
}

/**
 * Shell-level run state. A placeholder seam: the run lifecycle (S2.2) and the
 * Kill Switch (S9.8) slices replace `IDLE_RUN_STATE` with IPC/event-bus values.
 */
export interface RunState {
  running: boolean;
  autoEnabled: boolean;
  lastRunAt: string | null;
  newCount: number;
}

export const IDLE_RUN_STATE: RunState = {
  running: false,
  autoEnabled: false,
  lastRunAt: null,
  newCount: 0,
};
