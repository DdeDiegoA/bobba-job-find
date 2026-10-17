import { useTranslation } from "react-i18next";
import type { RateRow } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { verdictLabel } from "./labels";
import { useFunnelStats } from "./use-stats";

/**
 * Tracker → Stats (S9.9, FR-97, SM-4/SM-5): Applications by Stage, reply rate
 * and interview rate, broken down by Source, Verdict and language. Every figure
 * is computed locally by the core; this screen only formats it.
 */
export default function TrackerStats() {
  const { t, i18n } = useTranslation("audit_stats");
  const { t: tError } = useTranslation();
  const { t: tTracker } = useTranslation("tracker");
  const stats = useFunnelStats();

  // A f64 crosses as `number | null` (specta cannot promise a finite float);
  // the core only sends a computed ratio, so a null reads as zero.
  const percent = (rate: number | null) =>
    new Intl.NumberFormat(i18n.language, {
      style: "percent",
      maximumFractionDigits: 0,
    }).format(rate ?? 0);

  const data = stats.data;
  const stageLabel = (stage: string) => {
    const key = `stage.${stage}`;
    const translated = tTracker(key);
    return translated === key ? stage : translated;
  };

  return (
    <section className="flex flex-col gap-6" aria-labelledby="tracker-stats-title">
      <header className="flex flex-col gap-2">
        <h2 id="tracker-stats-title" className="font-display text-xl text-fg">
          {t("stats.title")}
        </h2>
        <p className="text-sm text-fg-muted">{t("stats.intro")}</p>
      </header>

      {stats.isPending ? <p className="text-sm text-fg-muted">{t("stats.loading")}</p> : null}
      {stats.error ? (
        <p role="alert" className="text-sm text-danger">
          {isAppError(stats.error) ? renderAppError(stats.error, tError) : t("stats.error")}
        </p>
      ) : null}

      {data ? (
        <>
          <dl className="grid grid-cols-2 gap-3 sm:grid-cols-4">
            <Figure label={t("stats.applications")} value={String(data.applications)} />
            <Figure label={t("stats.applied")} value={String(data.applied)} />
            <Figure label={t("stats.reply_rate")} value={percent(data.reply_rate)} />
            <Figure label={t("stats.interview_rate")} value={percent(data.interview_rate)} />
          </dl>

          {data.applications === 0 ? (
            <p className="text-sm text-fg-muted">{t("stats.empty")}</p>
          ) : (
            <>
              <div className="flex flex-col gap-2">
                <h3 className="font-display text-lg text-fg">{t("stats.by_stage")}</h3>
                <table className="w-full border-collapse text-left text-sm">
                  <thead>
                    <tr className="border-b border-border text-xs text-fg-muted">
                      <th scope="col" className="py-2 pr-3 font-medium">
                        {t("stats.column.stage")}
                      </th>
                      <th scope="col" className="py-2 font-medium">
                        {t("stats.column.count")}
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {data.by_stage.map((row) => (
                      <tr key={row.stage} className="border-b border-border/60">
                        <td className="py-2 pr-3 text-fg">{stageLabel(row.stage)}</td>
                        <td className="py-2 text-fg-muted">{row.count}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>

              <Breakdown
                title={t("stats.by_source")}
                rows={data.by_source}
                label={(row) =>
                  row.key === "unknown" ? t("stats.group.unknown") : (row.label ?? row.key)
                }
                t={t}
                percent={percent}
              />
              <Breakdown
                title={t("stats.by_verdict")}
                rows={data.by_verdict}
                label={(row) => verdictLabel(t, row.key)}
                t={t}
                percent={percent}
              />
              <Breakdown
                title={t("stats.by_language")}
                rows={data.by_language}
                label={(row) => row.key.toUpperCase()}
                t={t}
                percent={percent}
              />
            </>
          )}
        </>
      ) : null}
    </section>
  );
}

function Figure({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-2xl border border-border bg-surface p-4">
      <dt className="text-xs text-fg-muted">{label}</dt>
      <dd className="font-display text-2xl text-fg">{value}</dd>
    </div>
  );
}

function Breakdown({
  title,
  rows,
  label,
  t,
  percent,
}: {
  title: string;
  rows: RateRow[];
  label: (row: RateRow) => string;
  t: (key: string) => string;
  percent: (rate: number | null) => string;
}) {
  return (
    <div className="flex flex-col gap-2">
      <h3 className="font-display text-lg text-fg">{title}</h3>
      {rows.length === 0 ? (
        <p className="text-sm text-fg-muted">{t("stats.group.none")}</p>
      ) : (
        <table className="w-full border-collapse text-left text-sm">
          <thead>
            <tr className="border-b border-border text-xs text-fg-muted">
              <th scope="col" className="py-2 pr-3 font-medium">
                {t("stats.column.group")}
              </th>
              <th scope="col" className="py-2 pr-3 font-medium">
                {t("stats.column.applied")}
              </th>
              <th scope="col" className="py-2 pr-3 font-medium">
                {t("stats.column.reply_rate")}
              </th>
              <th scope="col" className="py-2 font-medium">
                {t("stats.column.interview_rate")}
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.key} className="border-b border-border/60">
                <td className="py-2 pr-3 text-fg">{label(row)}</td>
                <td className="py-2 pr-3 text-fg-muted">{row.applications}</td>
                <td className="py-2 pr-3 text-fg-muted">{percent(row.reply_rate)}</td>
                <td className="py-2 text-fg-muted">{percent(row.interview_rate)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
