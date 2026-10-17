import { NAV_ITEMS, type NavItem } from "@/app/shell/nav";
import type { OfferCardView } from "@/lib/ipc/bindings";

/**
 * What the command palette can reach (S6.7, EXPERIENCE §6): every page and
 * every Offer.
 *
 * Pages come from the shell's own navigation table, so the palette can never
 * offer a destination the sidebar does not have. Offers come from the same core
 * list the Inbox draws (`offers_inbox`), matched on title and company.
 */

/** One row of the palette. */
export interface PaletteEntry {
  /** Stable id; the active row is announced by it (`aria-activedescendant`). */
  id: string;
  kind: "page" | "offer";
  label: string;
  /** The line under the label; the company for an Offer, `null` else. */
  detail: string | null;
  /** Where choosing the row goes (react-router). */
  to: string;
}

/** The pages, in sidebar order. `t` resolves the `shell:nav.*` keys. */
export function pageEntries(
  t: (key: string) => string,
  items: readonly NavItem[] = NAV_ITEMS,
): PaletteEntry[] {
  return items.map((item) => ({
    id: `page:${item.to}`,
    kind: "page",
    label: t(item.labelKey),
    detail: null,
    to: item.to,
  }));
}

/** The Offers, newest first, minus the ones already dismissed (FR-66). */
export function offerEntries(cards: readonly OfferCardView[], untitled: string): PaletteEntry[] {
  return cards
    .filter((card) => !card.dismissed)
    .map((card) => ({
      id: `offer:${card.posting_id}`,
      kind: "offer" as const,
      label: card.title || untitled,
      detail: card.company || null,
      to: `/offer/${card.posting_id}`,
    }));
}

/** Case- and accent-insensitive text, so "espanol" finds "Español" (FR-114). */
export function normalize(text: string): string {
  return text.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase().trim();
}

/** Higher is a better match; 0 drops the row. */
function score(entry: PaletteEntry, needle: string): number {
  const label = normalize(entry.label);
  const detail = normalize(entry.detail ?? "");
  if (label === needle) return 4;
  if (label.startsWith(needle)) return 3;
  if (label.includes(needle)) return 2;
  if (detail.includes(needle)) return 1;
  return 0;
}

/**
 * The rows matching `query`, best first; an empty query keeps every row in its
 * given order (the page list). Equal scores keep their order (stable sort).
 */
export function rank(entries: readonly PaletteEntry[], query: string): PaletteEntry[] {
  const needle = normalize(query);
  if (!needle) return [...entries];
  return entries
    .map((entry) => ({ entry, score: score(entry, needle) }))
    .filter(({ score: value }) => value > 0)
    .sort((a, b) => b.score - a.score)
    .map(({ entry }) => entry);
}
