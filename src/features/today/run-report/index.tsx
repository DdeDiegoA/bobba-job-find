import { useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, RefreshCw } from "lucide-react";
import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import type { SourceReportView } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { cn } from "@/lib/utils";
import {
  failedSources,
  formatDuration,
  runReportKey,
  useLatestRunReport,
  useRetryRunReport,
  useRunProgress,
} from "./use-run-report";

/**
 * The last search run's report (S5.2, FR-46, NFR-9, NFR-14): what every Source
 * returned, what ingestion made of it, what scoring found, and how long it took.
 *
 * A run that could not reach some Sources is not a failure: the panel says
 * which ones and offers Retry (AC3), and the rest of the report still reads.
 */
export default function TodayRunReport() {
  const { t } = useTranslation("run_report");
  const { t: tError } = useTranslation();
  const report = useLatestRunReport();
  const retry = useRetryRunReport();
  const queryClient = useQueryClient();
  const reload = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: runReportKey });
  }, [queryClient]);
  const progress = useRunProgress(reload);

  if (report.isPending) {
    return (
      <section aria-busy="true" className="rounded-card border border-border bg-surface p-4">
        <span className="sr-only">{t("loading")}</span>
        <div className="h-20 animate-pulse rounded-card bg-surface-muted" />
      </section>
    );
  }

  // A read that failed is not "no run yet": show what went wrong instead of an
  // empty state that would hide it.
  if (report.isError) {
    const error = isAppError(report.error) ? renderAppError(report.error, tError) : null;
    return (
      <section className="rounded-card border border-border bg-surface p-4">
        <p className="text-sm text-danger">{error ?? tError("errors.unknown")}</p>
      </section>
    );
  }

  const data = report.data;
  if (!data) {
    return <EmptyState title={t("empty.title")} description={t("empty.body")} />;
  }

  const failed = failedSources(data);
  const retryError =
    retry.error && isAppError(retry.error) ? renderAppError(retry.error, tError) : null;

  return (
    <section
      aria-labelledby="run-report-title"
      aria-busy={progress !== null}
      className={cn(
        "flex flex-col gap-4 rounded-card border border-border bg-surface p-4",
        progress !== null && "opacity-60",
      )}
    >
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h2 id="run-report-title" className="font-display text-xl text-fg">
          {t("title")}
        </h2>
        <p className="text-sm text-fg-muted">
          {t("started", { when: new Date(data.started_at).toLocaleString() })} ·{" "}
          {t("duration", { duration: formatDuration(data.duration_ms) })}
        </p>
      </div>

      {data.partial ? (
        <div
          role="alert"
          data-slot="partial-failure"
          className="flex flex-col gap-2 rounded-card border border-warning/40 bg-warning/10 p-3"
        >
          <p className="flex items-center gap-2 text-sm text-fg">
            <AlertTriangle aria-hidden className="size-4 shrink-0 text-warning" />
            {t("partial.title", { count: failed.length, total: data.queried })}
          </p>
          <ul className="flex flex-col gap-1 text-sm text-fg-muted">
            {failed.map((source) => (
              <li key={source.source_id}>
                <span className="font-medium text-fg">{source.name}</span>
                {source.error ? <span className="text-fg-subtle"> — {source.error}</span> : null}
              </li>
            ))}
          </ul>
          <div className="flex items-center gap-3">
            <Button
              size="sm"
              variant="outline"
              disabled={retry.isPending}
              onClick={() => retry.mutate()}
            >
              <RefreshCw aria-hidden className="size-4" />
              {retry.isPending ? t("partial.retrying") : t("partial.retry")}
            </Button>
            {retryError ? <span className="text-sm text-danger">{retryError}</span> : null}
          </div>
        </div>
      ) : null}

      <dl className="grid grid-cols-2 gap-3 sm:grid-cols-4 lg:grid-cols-7">
        <Stat label={t("totals.queried")} value={data.queried} />
        <Stat label={t("totals.fetched")} value={data.fetched} />
        <Stat label={t("totals.new")} value={data.new} />
        <Stat label={t("totals.duplicates")} value={data.duplicates} />
        <Stat label={t("totals.vetoed")} value={data.vetoed} />
        <Stat label={t("totals.ranked")} value={data.ranked} />
        <Stat label={t("totals.agentTasks")} value={data.agent_tasks_used} />
      </dl>

      <div className="flex flex-col gap-2">
        <h3 className="font-display text-lg text-fg">{t("sources.title")}</h3>
        <ul className="flex flex-col gap-1" data-slot="run-sources">
          {data.sources.map((source) => (
            <SourceLine key={source.source_id} source={source} />
          ))}
        </ul>
      </div>

      {data.errors.length > 0 ? (
        <details className="text-sm text-fg-muted">
          <summary>{t("errors.title", { count: data.errors.length })}</summary>
          <ul className="mt-1 flex flex-col gap-1">
            {data.errors.map((error) => (
              <li key={error}>{error}</li>
            ))}
          </ul>
        </details>
      ) : null}
    </section>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-card bg-surface-muted px-3 py-2">
      <dt className="text-xs text-fg-subtle">{label}</dt>
      <dd className="font-display text-lg text-fg">{value}</dd>
    </div>
  );
}

function SourceLine({ source }: { source: SourceReportView }) {
  const { t } = useTranslation("run_report");
  return (
    <li
      className="flex flex-wrap items-baseline justify-between gap-2 rounded-xl bg-surface-muted px-3 py-2 text-sm"
      data-outcome={source.outcome}
    >
      <span className={cn("font-medium", source.outcome === "failed" ? "text-danger" : "text-fg")}>
        {source.name}
      </span>
      <span className="text-fg-muted">
        {source.outcome === "failed"
          ? t("sources.failed")
          : t("sources.counts", {
              fetched: source.fetched,
              new: source.new,
              duplicates: source.duplicates,
            })}
      </span>
    </li>
  );
}
