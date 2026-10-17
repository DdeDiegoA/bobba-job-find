import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import { axisAnchor, type Detail, VERDICT_LABELS, verdictId } from "./detail";

/**
 * The DualScore (FR-55/FR-56/FR-57, EXPERIENCE §9.4): Fit, Quality and the Rank
 * they combine into.
 *
 * AC3: **Fit and Quality are buttons** — clicking one scrolls the page to that
 * axis's factor group, so the number and the evidence that produced it are never
 * far apart. Rank is a display only: there is no rank group to jump to.
 *
 * A dash is shown when the axis is `null` (AD-8: a missing reading is unknown,
 * never a zero), and a Lead says so instead of pretending to be a band.
 */
export function DualScore({ detail }: { detail: Detail }) {
  const { t } = useTranslation("offer");
  const verdict = verdictId(detail);

  const scrollTo = (axis: "fit" | "quality") => {
    // The group is on this same screen (AC3); a missing one means nothing was
    // read for that axis, which the dash already says.
    document
      .getElementById(axisAnchor(axis))
      ?.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  return (
    <div className="flex flex-wrap items-stretch gap-3">
      <Axis
        axis="fit"
        label={t("fit")}
        value={detail.fit}
        hint={t("dual.scroll_to_axis", { axis: t("fit") })}
        onClick={() => scrollTo("fit")}
      />
      <Axis
        axis="quality"
        label={t("quality")}
        value={detail.quality}
        hint={t("dual.scroll_to_axis", { axis: t("quality") })}
        onClick={() => scrollTo("quality")}
      />

      <div className="flex min-w-28 flex-col justify-center rounded-card border border-border bg-surface px-4 py-3">
        <span className="text-xs text-fg-subtle">{t("rank")}</span>
        <span className="font-display text-2xl text-fg">
          {detail.rank === null ? "—" : Math.round(detail.rank)}
        </span>
        {verdict && VERDICT_LABELS[verdict] ? (
          <span
            className={cn("text-xs font-medium", verdict === "lead" ? "text-fg-muted" : "text-fg")}
          >
            {t(VERDICT_LABELS[verdict])}
          </span>
        ) : (
          <span className="text-xs text-fg-subtle">{t("unscored")}</span>
        )}
      </div>

      <div className="flex min-w-28 flex-col justify-center rounded-card border border-border bg-surface px-4 py-3">
        <span className="text-xs text-fg-subtle">{t("confidence.title")}</span>
        <span className="text-sm font-medium text-fg">
          {detail.confidence ? t(`offer:confidence.${detail.confidence}`) : "—"}
        </span>
      </div>
    </div>
  );
}

/** One clickable axis of the DualScore (AC3). */
function Axis({
  axis,
  label,
  value,
  hint,
  onClick,
}: {
  axis: "fit" | "quality";
  label: string;
  value: number | null;
  hint: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      data-axis={axis}
      aria-label={hint}
      onClick={onClick}
      className="flex min-w-28 flex-col justify-center rounded-card border border-border bg-surface px-4 py-3 text-left hover:border-focus focus-visible:outline-2 focus-visible:outline-focus"
    >
      <span className="text-xs text-fg-subtle">{label}</span>
      <span className="font-display text-2xl text-fg">
        {value === null ? "—" : Math.round(value)}
      </span>
      <span className="text-xs text-fg-subtle">{hint}</span>
    </button>
  );
}
