import { useQuery } from "@tanstack/react-query";
import { commands, type OfferCardView } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/**
 * The Inbox's read model (S6.4, FR-63/FR-64).
 *
 * The core hands back **one flat list of cards** (`offers_inbox`): every Offer
 * with its newest ScoreCard, its Sources, its badges and, when a Gate vetoed it,
 * the Gate and its quote. Everything this file does is a way of *showing* that
 * list — the sections of FR-63, the five filters of FR-63, the four sorts. None
 * of it invents a score or a band: a card with no ScoreCard stays "not scored
 * yet" here exactly as it arrives.
 *
 * The one rule the sections share: an Offer a Gate vetoed appears **only** under
 * Excluded (a VETO is not a ranked Offer, FR-54), while an Offer closing soon
 * appears under its band **and** under Closing soon (§9.3 shows both).
 */

/** One Offer card, as the core sends it. */
export type Card = OfferCardView;

/** The Inbox is one query: every card, newest first. */
export const inboxKey = ["offers", "inbox"] as const;

/** Every Offer card, newest first. */
export function useInboxOffers() {
  return useQuery({
    queryKey: inboxKey,
    queryFn: () => commands.offersInbox().then(unwrap),
  });
}

/** The sections of FR-63, in the order the Inbox draws them. */
export type SectionId = "strong" | "good" | "moderate" | "weak_poor" | "closing_soon" | "excluded";

export interface Section {
  id: SectionId;
  /** i18n key under the `inbox` namespace. */
  labelKey: string;
  /** Collapsed until the user opens it (FR-63: Weak/Poor starts collapsed). */
  collapsed: boolean;
}

export const SECTIONS: readonly Section[] = [
  { id: "strong", labelKey: "inbox:section.strong", collapsed: false },
  { id: "good", labelKey: "inbox:section.good", collapsed: false },
  { id: "moderate", labelKey: "inbox:section.moderate", collapsed: false },
  { id: "weak_poor", labelKey: "inbox:section.weak_poor", collapsed: true },
  { id: "closing_soon", labelKey: "inbox:section.closing_soon", collapsed: false },
  { id: "excluded", labelKey: "inbox:section.excluded", collapsed: false },
];

/** FR-54: a vetoed Offer is Excluded, never ranked. */
export function isExcluded(card: Card): boolean {
  return card.exclusion !== null;
}

/**
 * The band a card belongs to. An Offer nobody has scored yet has no Verdict, and
 * is shown with the ones that scored low: it is the group a user opens when they
 * are looking for what is left, and it never claims a band it does not have.
 */
export function bandOf(card: Card): SectionId {
  switch (card.verdict) {
    case "strong":
      return "strong";
    case "good":
      return "good";
    case "moderate":
      return "moderate";
    default:
      return "weak_poor";
  }
}

/** The cards one section shows (§9.3: band, then Closing soon, then Excluded). */
export function cardsIn(cards: readonly Card[], section: SectionId): Card[] {
  if (section === "excluded") {
    return cards.filter(isExcluded);
  }
  const ranked = cards.filter((card) => !isExcluded(card));
  if (section === "closing_soon") {
    return ranked.filter((card) => card.closing_soon);
  }
  if (section === "weak_poor") {
    return ranked.filter((card) => bandOf(card) === "weak_poor");
  }
  return ranked.filter((card) => bandOf(card) === section);
}

/** How many cards each Verdict band holds (Today's "3 Strong · 5 Good …"). */
export function bandCounts(cards: readonly Card[]): {
  strong: number;
  good: number;
  moderate: number;
  weak_poor: number;
  excluded: number;
} {
  const counts = { strong: 0, good: 0, moderate: 0, weak_poor: 0, excluded: 0 };
  for (const card of cards) {
    if (isExcluded(card)) {
      counts.excluded += 1;
    } else {
      counts[bandOf(card) as "strong" | "good" | "moderate" | "weak_poor"] += 1;
    }
  }
  return counts;
}

/** The sorts of FR-63: Rank (the default), date, Fit, Quality. */
export type SortKey = "rank" | "date" | "fit" | "quality";

export const SORTS: readonly { key: SortKey; labelKey: string }[] = [
  { key: "rank", labelKey: "inbox:sort.rank" },
  { key: "date", labelKey: "inbox:sort.date" },
  { key: "fit", labelKey: "inbox:sort.fit" },
  { key: "quality", labelKey: "inbox:sort.quality" },
];

/** The date a card is sorted (and aged) by: when it was posted, else first seen. */
export function cardDate(card: Card): string {
  return card.posted_at ?? card.first_seen;
}

/** The number a card is sorted by, `null` when it has none. */
function sortValue(card: Card, key: Exclude<SortKey, "date">): number | null {
  if (key === "rank") return card.rank;
  if (key === "fit") return card.fit;
  return card.quality;
}

/**
 * Sort a copy of the cards. Highest first — Rank, Fit and Quality are
 * higher-is-better, and "date" is newest first. A card with no number for the
 * chosen sort goes **last** rather than being shown as a zero (AD-8: nothing is
 * imputed); equal values keep the core's newest-first order.
 */
export function sortCards(cards: readonly Card[], key: SortKey): Card[] {
  const sorted = [...cards];
  if (key === "date") {
    sorted.sort(
      (a, b) => cardDate(b).localeCompare(cardDate(a)) || a.posting_id.localeCompare(b.posting_id),
    );
    return sorted;
  }
  sorted.sort((a, b) => {
    const left = sortValue(a, key);
    const right = sortValue(b, key);
    if (left === null && right === null) return 0;
    if (left === null) return 1;
    if (right === null) return -1;
    if (right !== left) return right - left;
    return 0;
  });
  return sorted;
}

/** The filters of FR-63. `null` means "any"; `salaryKnown` is a toggle. */
export interface Filters {
  source: string | null;
  workMode: string | null;
  lang: string | null;
  salaryKnown: boolean;
  verdict: string | null;
}

export const NO_FILTERS: Filters = {
  source: null,
  workMode: null,
  lang: null,
  salaryKnown: false,
  verdict: null,
};

/** Whether any filter is on (the filtered-empty state offers Clear only then). */
export function hasFilters(filters: Filters): boolean {
  return (
    filters.source !== null ||
    filters.workMode !== null ||
    filters.lang !== null ||
    filters.salaryKnown ||
    filters.verdict !== null
  );
}

/**
 * The cards the filters keep, in the core's order.
 *
 * The Verdict filter compares the Verdict the card **shows** ([`verdictId`]):
 * FR-57 draws a Low Confidence card as a Lead, so "Lead" is one of the choices
 * and a card shown as a Lead is not also reachable under its band.
 */
export function filterCards(cards: readonly Card[], filters: Filters): Card[] {
  return cards.filter((card) => {
    if (filters.source !== null && !card.sources.includes(filters.source)) return false;
    if (filters.workMode !== null && card.work_mode !== filters.workMode) return false;
    if (filters.lang !== null && card.lang !== filters.lang) return false;
    if (filters.salaryKnown && card.amount_minor === null) return false;
    if (filters.verdict !== null && verdictId(card) !== filters.verdict) return false;
    return true;
  });
}

/** The Verdict choices of FR-63: the five bands, plus the Lead FR-57 shows. */
export const VERDICT_FILTERS: readonly string[] = [
  "strong",
  "good",
  "moderate",
  "weak",
  "poor",
  "lead",
];

/** The choices each filter offers, from the cards themselves (sorted, unique). */
export function filterOptions(cards: readonly Card[]): {
  sources: string[];
  workModes: string[];
  langs: string[];
} {
  const sources = new Set<string>();
  const workModes = new Set<string>();
  const langs = new Set<string>();
  for (const card of cards) {
    for (const source of card.sources) sources.add(source);
    workModes.add(card.work_mode);
    langs.add(card.lang);
  }
  return {
    sources: [...sources].sort(),
    workModes: [...workModes].sort(),
    langs: [...langs].sort(),
  };
}

/** Whole days between a card's date and `now`, never negative (a clock skew is 0). */
export function ageDays(card: Card, now: Date = new Date()): number {
  const then = Date.parse(cardDate(card));
  if (Number.isNaN(then)) return 0;
  return Math.max(0, Math.floor((now.getTime() - then) / 86_400_000));
}

/** The verdict id a card's chip shows: `lead` when it is a Lead (FR-57). */
export function verdictId(card: Card): string | null {
  if (card.verdict === null) return null;
  return card.lead ? "lead" : card.verdict;
}

/** Wording held in data, as resolvable keys (AD-21): a rename fails the build. */
export const VERDICT_LABELS: Record<string, string> = {
  strong: "inbox:verdict.strong",
  good: "inbox:verdict.good",
  moderate: "inbox:verdict.moderate",
  weak: "inbox:verdict.weak",
  poor: "inbox:verdict.poor",
  lead: "inbox:verdict.lead",
};

export const WORK_MODE_LABELS: Record<string, string> = {
  remote: "inbox:work_mode.remote",
  hybrid: "inbox:work_mode.hybrid",
  onsite: "inbox:work_mode.onsite",
  unknown: "inbox:work_mode.unknown",
};

export const BADGE_LABELS: Record<string, string> = {
  lead: "inbox:badge.lead",
  closing_soon: "inbox:badge.closing_soon",
  expired: "inbox:badge.expired",
  below_floor: "inbox:badge.below_floor",
  eligibility_not_met: "inbox:badge.eligibility_not_met",
  eligibility_unverified: "inbox:badge.eligibility_unverified",
  language_level: "inbox:badge.language_level",
  language_unverified: "inbox:badge.language_unverified",
  location_not_met: "inbox:badge.location_not_met",
  location_unverified: "inbox:badge.location_unverified",
  compensation_unverified: "inbox:badge.compensation_unverified",
  check: "inbox:badge.check",
};

export const EXCLUSION_REASON_LABELS: Record<string, string> = {
  requirement_not_met: "inbox:exclusion.reason.requirement_not_met",
  below_required_level: "inbox:exclusion.reason.below_required_level",
  below_floor: "inbox:exclusion.reason.below_floor",
  expired: "inbox:exclusion.reason.expired",
  unverified: "inbox:exclusion.reason.unverified",
  closing_soon: "inbox:exclusion.reason.closing_soon",
  satisfied: "inbox:exclusion.reason.satisfied",
};

/** The pay a card shows, or `null` when the posting named none (FR-64). */
export function salaryLabel(card: Card, locale: string): string | null {
  if (card.amount_minor === null || card.currency === null) return null;
  const amount = new Intl.NumberFormat(locale, {
    style: "currency",
    currency: card.currency,
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(card.amount_minor / 100);
  return amount;
}
