import { Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useRunProgress } from "./use-run-report";

/**
 * The topbar's live search progress (S5.2 AC1): which Source is being queried
 * and how many of the run's Sources are done. Nothing renders between runs.
 */
export function RunProgress() {
  const { t } = useTranslation("run_report");
  const progress = useRunProgress();

  if (!progress) return null;

  return (
    <span
      data-slot="run-progress"
      role="status"
      className="flex items-center gap-1.5 text-xs text-fg-muted max-lg:hidden"
    >
      <Loader2 aria-hidden className="size-3.5 animate-spin" />
      {progress.sourceName
        ? t("progress.source", {
            name: progress.sourceName,
            done: progress.done,
            total: progress.total,
          })
        : t("progress.starting", { total: progress.total })}
    </span>
  );
}
