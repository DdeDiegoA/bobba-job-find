import { useQuery } from "@tanstack/react-query";
import { majorUnits } from "@/lib/format";
import { commands, type FactorView, type OfferDetailView } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/**
 * The Offer detail's read model (S6.5, FR-65, EXPERIENCE §9.4).
 *
 * The core sends one `OfferDetailView`: the sanitised posting, the Gates with
 * their quotes, the eleven factors, the Sources and merged copies, the Apply
 * Channels and — when the agent wrote one — the Explanation in the UI language.
 * This file only names things and groups them; it invents no score (AD-8).
 */

/** One Offer in full, as the core sends it. */
export type Detail = OfferDetailView;

/** The factors of one axis, in the order the card stores them. */
export interface FactorGroup {
  axis: "fit" | "quality";
  /** The element id the DualScore scrolls to (AC3). */
  anchor: string;
  labelKey: string;
  factors: FactorView[];
}

/** The two groups of the breakdown, Fit then Quality (FR-55/FR-56). */
export const AXES: readonly { axis: "fit" | "quality"; labelKey: string }[] = [
  { axis: "fit", labelKey: "offer:factors.fit" },
  { axis: "quality", labelKey: "offer:factors.quality" },
];

/** The DOM id of one axis's factor group (AC3: the DualScore scrolls to it). */
export function axisAnchor(axis: "fit" | "quality"): string {
  return `offer-factor-${axis}`;
}

/** The factor groups a detail draws, each with its own factors. */
export function factorGroups(detail: Detail): FactorGroup[] {
  return AXES.map(({ axis, labelKey }) => ({
    axis,
    anchor: axisAnchor(axis),
    labelKey,
    factors: detail.factors.filter((factor) => factor.axis === axis),
  }));
}

/** The factor labels, from the `offer` namespace. */
export const FACTOR_LABELS: Record<string, string> = {
  f1_skills: "offer:factor.f1_skills",
  f2_experience: "offer:factor.f2_experience",
  f3_semantic: "offer:factor.f3_semantic",
  f4_location: "offer:factor.f4_location",
  f5_education: "offer:factor.f5_education",
  f6_language: "offer:factor.f6_language",
  q1_compensation: "offer:factor.q1_compensation",
  q2_company: "offer:factor.q2_company",
  q3_terms: "offer:factor.q3_terms",
  q4_legitimacy: "offer:factor.q4_legitimacy",
  q5_alignment: "offer:factor.q5_alignment",
};

/** The Gate labels, from the `offer` namespace. */
export const GATE_LABELS: Record<string, string> = {
  eligibility: "offer:gate.eligibility",
  language: "offer:gate.language",
  location: "offer:gate.location",
  compensation: "offer:gate.compensation",
  deadline: "offer:gate.deadline",
};

/** The Verdict band labels, from the `offer` namespace. */
export const VERDICT_LABELS: Record<string, string> = {
  strong: "offer:verdict.strong",
  good: "offer:verdict.good",
  moderate: "offer:verdict.moderate",
  weak: "offer:verdict.weak",
  poor: "offer:verdict.poor",
  lead: "offer:verdict.lead",
};

/** The Apply Channel labels (FR-81), from the `offer` namespace. */
export const CHANNEL_LABELS: Record<string, string> = {
  email_apply: "offer:channel.email_apply",
  ats_form: "offer:channel.ats_form",
  manual: "offer:channel.manual",
};

/** The work-mode labels, from the `offer` namespace. */
export const WORK_MODE_LABELS: Record<string, string> = {
  remote: "offer:work_mode.remote",
  hybrid: "offer:work_mode.hybrid",
  onsite: "offer:work_mode.onsite",
  unknown: "offer:work_mode.unknown",
};

/** The pay-period labels, from the `offer` namespace. */
export const PAY_PERIOD_LABELS: Record<string, string> = {
  year: "offer:pay.year",
  month: "offer:pay.month",
  hour: "offer:pay.hour",
};

/**
 * The Verdict a detail draws: FR-57's Lead replaces the band a Low Confidence
 * card could not honestly claim (`null` when the Offer has no card at all).
 */
export function verdictId(detail: Detail): string | null {
  if (!detail.verdict) return null;
  return detail.lead ? "lead" : detail.verdict;
}

/** The pay, formatted in the user's locale, `null` when the posting named none. */
export function payAmount(detail: Detail, locale: string): string | null {
  if (detail.amount_minor === null || !detail.currency) return null;
  const format = new Intl.NumberFormat(locale, {
    style: "currency",
    currency: detail.currency,
    maximumFractionDigits: 0,
  });
  // Minor units, scaled by the currency's own exponent (AD-17).
  return format.format(majorUnits(detail.amount_minor, detail.currency));
}

/** Whole days since the Offer was posted (or first seen), never negative. */
export function ageDays(detail: Detail, now: Date = new Date()): number {
  const then = Date.parse(detail.posted_at ?? detail.first_seen);
  if (Number.isNaN(then)) return 0;
  return Math.max(0, Math.floor((now.getTime() - then) / 86_400_000));
}

/**
 * Defensive remote-content strip (FR-65, AD-7).
 *
 * The core sanitises the posting before it is stored — `description_html` is the
 * only form the UI may render — and this drops the elements and attributes that
 * could still fetch something when rendered, so the detail never loads remote
 * content even if a future sanitiser regression let one through.
 */
export function withoutRemoteContent(html: string): string {
  return html
    .replace(/<\/?\s*(img|iframe|video|audio|source|embed|object|link)\b[^>]*>/gi, "")
    .replace(/\s(src|srcset|poster|background)\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)/gi, "");
}

/** The detail query: one Offer, keyed by its id and the UI language (FR-59). */
export function useOfferDetail(postingId: string, lang: string) {
  return useQuery({
    queryKey: ["offers", "detail", postingId, lang],
    queryFn: () => commands.offerDetail(postingId, lang).then(unwrap),
    enabled: postingId.length > 0,
  });
}

/** The language the narration is asked for: the supported UI language. */
export function uiLanguage(language: string | undefined): "en" | "es" {
  return language?.toLowerCase().startsWith("es") ? "es" : "en";
}
