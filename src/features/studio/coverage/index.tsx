import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";

/**
 * One key requirement of the Offer with what the Profile backs of it (FR-75), as
 * the coverage panel reads it. The lists come from `boba_domain::keywords`
 * through the studio's command: this panel never matches a keyword itself, so
 * what it shows and what the ATS check requires cannot drift apart.
 */
export type CoverageRequirement = {
  /** The requirement as the Offer states it, so the user recognises it. */
  requirement: string;
  /** Keywords a Profile fact backs; the ATS check requires them in the PDF. */
  covered: string[];
  /** Keywords nothing backs: the declared gaps (AD-15). */
  gaps: string[];
};

export type StudioCoverageProps = {
  /** Every key requirement the Offer states, in the Offer's own order. */
  requirements: CoverageRequirement[];
};

/**
 * Keyword coverage and declared gaps (FR-75, AC2). Every requirement is shown as
 * covered or a gap, and the gaps that were **not** added are named: Boba never
 * writes a keyword the Profile does not back (AD-15).
 */
export default function StudioCoverage({ requirements }: StudioCoverageProps) {
  const { t } = useTranslation("studio_coverage");

  const covered = requirements.reduce((count, item) => count + item.covered.length, 0);
  const total = requirements.reduce(
    (count, item) => count + item.covered.length + item.gaps.length,
    0,
  );
  // The same keyword can be missing from two requirements; the panel names it once.
  const gaps = [...new Set(requirements.flatMap((item) => item.gaps))];

  return (
    <section
      className="flex flex-col gap-3 rounded-2xl border border-border bg-surface p-4"
      aria-labelledby="studio-coverage-title"
      data-gaps={gaps.length}
    >
      <div className="flex items-baseline justify-between gap-2">
        <h2 id="studio-coverage-title" className="font-display text-lg text-fg">
          {t("title")}
        </h2>
        <span className="shrink-0 text-xs text-fg-muted" data-covered={covered} data-total={total}>
          {t("count", { covered, total })}
        </span>
      </div>
      <p className="text-xs text-fg-subtle">{t("hint")}</p>
      <p className="text-sm text-fg-muted" role="status">
        {gaps.length === 0
          ? t("status.all", { total })
          : t("status.partial", { gaps: gaps.length })}
      </p>

      <ul className="flex flex-col gap-2">
        {requirements.map((item, index) => {
          const missing = item.gaps.length > 0;
          return (
            <li
              // biome-ignore lint/suspicious/noArrayIndexKey: a requirement's identity is its position in the Offer, which the core also addresses by index
              key={index}
              data-coverage={missing ? "gap" : "covered"}
              className={cn(
                "flex flex-col gap-1 rounded-xl border-l-2 px-2 py-1",
                missing ? "border-l-warning bg-surface-muted" : "border-l-transparent",
              )}
            >
              <div className="flex items-start gap-2">
                <span className="min-w-0 flex-1 text-sm text-fg">{item.requirement}</span>
                <span
                  className={cn(
                    "shrink-0 rounded-xl px-2 py-0.5 text-xs",
                    missing ? "bg-warning/10 text-warning" : "bg-primary/10 text-primary",
                  )}
                >
                  {missing ? t("gap") : t("covered")}
                </span>
              </div>
              {missing ? (
                <p className="text-xs text-warning">
                  {t("missing", { keywords: item.gaps.join(", ") })}
                </p>
              ) : (
                <p className="text-xs text-fg-muted">{item.covered.join(", ")}</p>
              )}
            </li>
          );
        })}
      </ul>

      {gaps.length > 0 ? (
        <p className="text-xs text-fg-muted" data-not-inserted="">
          {t("notInserted", { keywords: gaps.join(", ") })}
        </p>
      ) : null}
    </section>
  );
}
