import {
  Ban,
  CircleHelp,
  Clock,
  Minus,
  Sparkles,
  Star,
  ThumbsDown,
  ThumbsUp,
  XCircle,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { cn } from "@/lib/utils";
import {
  ageDays,
  BADGE_LABELS,
  type Card,
  EXCLUSION_REASON_LABELS,
  salaryLabel,
  VERDICT_LABELS,
  verdictId,
  WORK_MODE_LABELS,
} from "./offers";

/**
 * One Offer card (FR-64, EXPERIENCE §9.3). It shows the title, the company and
 * where the job is, the pay (only when the posting named one), the Fit and
 * Quality axes, the Verdict as **text plus an icon** (never colour alone,
 * NFR-15), the age, the badges the Gates earned (FR-60) and the Sources.
 *
 * FR-57: a card whose Confidence is Low is shown as a **Lead** — dashed, "worth
 * checking" — never as a hard band. An Offer nobody scored yet says so
 * ("Not scored yet") instead of showing a zero (AD-8: nothing is imputed).
 *
 * A vetoed Offer (FR-54) shows the Gate that vetoed it and its quote, so the
 * Excluded section never silently drops an Offer.
 */

/** The icon each Verdict chip carries (text + icon, NFR-15). */
const VERDICT_ICONS = {
  strong: Star,
  good: ThumbsUp,
  moderate: Minus,
  weak: ThumbsDown,
  poor: XCircle,
  lead: Sparkles,
} as const;

/** The tone each Verdict chip carries. Every chip also carries its word. */
const VERDICT_TONES: Record<string, string> = {
  strong: "border-band-strong text-band-strong",
  good: "border-band-good text-band-good",
  moderate: "border-band-moderate text-band-moderate",
  weak: "border-band-weak text-band-weak",
  poor: "border-band-poor text-band-poor",
  // FR-57: a Lead is dashed — visibly "not a judgement".
  lead: "border-dashed border-fg-subtle text-fg-muted",
};

/** The Gate labels the Excluded line reads. */
const GATE_LABELS: Record<string, string> = {
  eligibility: "inbox:gate.eligibility",
  language: "inbox:gate.language",
  location: "inbox:gate.location",
  compensation: "inbox:gate.compensation",
  deadline: "inbox:gate.deadline",
};

const SALARY_PERIOD_LABELS: Record<string, string> = {
  year: "inbox:salary_period.year",
  month: "inbox:salary_period.month",
  hour: "inbox:salary_period.hour",
};

/** One axis of the DualScore (FR-55/FR-56): the number, or a dash when unknown. */
function Axis({ label, value }: { label: string; value: number | null }) {
  return (
    <span className="inline-flex items-center gap-1 rounded-full border border-border px-2 py-0.5 text-xs text-fg-muted">
      <span>{label}</span>
      <span className="font-medium text-fg">{value === null ? "—" : Math.round(value)}</span>
    </span>
  );
}

export function OfferCard({ card }: { card: Card }) {
  const { t, i18n } = useTranslation("inbox");
  const verdict = verdictId(card);
  const VerdictIcon = verdict ? VERDICT_ICONS[verdict as keyof typeof VERDICT_ICONS] : null;
  const salary = salaryLabel(card, i18n.language);
  const age = ageDays(card);
  const workMode = WORK_MODE_LABELS[card.work_mode] ?? WORK_MODE_LABELS.unknown;

  return (
    <article
      className={cn(
        "flex flex-col gap-2 rounded-card border bg-surface p-4",
        card.exclusion ? "border-danger/40" : "border-border",
      )}
    >
      <Link
        to={`/offer/${card.posting_id}`}
        aria-label={t("card.open", { title: card.title || t("card.untitled") })}
        className="flex flex-col gap-2 rounded-xl focus-visible:outline-2 focus-visible:outline-focus"
      >
        <div className="flex flex-wrap items-start justify-between gap-x-3 gap-y-2">
          <div className="min-w-0">
            <h3 className="truncate font-display text-base text-fg">
              {card.title || t("card.untitled")}
            </h3>
            <p className="truncate text-sm text-fg-muted">
              {[card.company || null, card.locations.join(", ") || null, t(workMode)]
                .filter(Boolean)
                .join(" · ")}
            </p>
          </div>

          <div className="flex shrink-0 flex-wrap items-center gap-1.5">
            <Axis label={t("card.fit")} value={card.fit} />
            <Axis label={t("card.quality")} value={card.quality} />
            {verdict && VerdictIcon ? (
              <span
                className={cn(
                  "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs font-medium",
                  VERDICT_TONES[verdict],
                )}
              >
                <VerdictIcon aria-hidden className="size-3" />
                <span>{t(VERDICT_LABELS[verdict])}</span>
                {card.rank !== null ? <span>{Math.round(card.rank)}</span> : null}
              </span>
            ) : (
              <span className="inline-flex items-center gap-1 rounded-full border border-dashed border-border px-2 py-0.5 text-xs text-fg-subtle">
                <CircleHelp aria-hidden className="size-3" />
                {t("card.unscored")}
              </span>
            )}
            {card.confidence ? (
              <span className="text-xs text-fg-subtle">
                {t(`inbox:card.confidence.${card.confidence}`)}
              </span>
            ) : null}
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-fg-subtle">
          {salary ? (
            <span className="text-fg-muted">
              {salary}
              {card.pay_period && SALARY_PERIOD_LABELS[card.pay_period]
                ? ` ${t(SALARY_PERIOD_LABELS[card.pay_period])}`
                : null}
            </span>
          ) : null}
          <span className="inline-flex items-center gap-1">
            <Clock aria-hidden className="size-3" />
            {t("card.age.days", { days: age })}
          </span>
          {card.sources.length > 0 ? (
            <span>{t("card.sources", { sources: card.sources.join(" · ") })}</span>
          ) : null}
        </div>

        {card.badges.length > 0 ? (
          <ul className="flex flex-wrap gap-1.5" aria-label={t("card.badges")}>
            {card.badges.map((badge) => (
              <li
                key={badge.id}
                className="rounded-full border border-warning px-2 py-0.5 text-xs text-warning"
              >
                {BADGE_LABELS[badge.id] ? t(BADGE_LABELS[badge.id]) : t("inbox:badge.check")}
              </li>
            ))}
          </ul>
        ) : null}
      </Link>

      {card.exclusion ? (
        <div className="flex flex-col gap-1 border-t border-danger/30 pt-2">
          <p className="inline-flex items-center gap-1 text-sm font-medium text-danger">
            <Ban aria-hidden className="size-3.5" />
            {t("exclusion.line", {
              gate: GATE_LABELS[card.exclusion.gate]
                ? t(GATE_LABELS[card.exclusion.gate])
                : card.exclusion.gate,
              reason: EXCLUSION_REASON_LABELS[card.exclusion.reason]
                ? t(EXCLUSION_REASON_LABELS[card.exclusion.reason])
                : card.exclusion.reason,
            })}
          </p>
          {/* The posting's own words: plain text, never markup (AD-7). */}
          {card.exclusion.quote ? (
            <p className="text-sm text-fg-muted">
              {t("exclusion.quote", { quote: card.exclusion.quote })}
            </p>
          ) : null}
        </div>
      ) : null}
    </article>
  );
}
