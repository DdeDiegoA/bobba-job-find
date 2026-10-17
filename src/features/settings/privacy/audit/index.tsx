import { save } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { AuditEntry } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { eventLabel, kindLabel, outcomeLabel } from "./labels";
import { entrySummary } from "./summary";
import { PAGE_SIZE, useAuditLog, useAuditLogExport } from "./use-audit-log";

/**
 * Settings → Privacy → Audit Log (S9.9, FR-89, SM-7). Every send, Auto
 * decision, Kill Switch event, opt-in and Stage change, with the hash-chain
 * verdict and a CSV export. Read-only: nothing here writes to the log.
 */
export default function SettingsPrivacyAudit() {
  const { t, i18n } = useTranslation("audit_stats");
  const { t: tError } = useTranslation();
  // The core paginates (CH12): the viewer keeps the cursor each page started
  // at, and never holds more than one page of rows.
  const [cursors, setCursors] = useState<(string | null)[]>([null]);
  const [page, setPage] = useState(0);
  const audit = useAuditLog(cursors[page] ?? null);
  const exported = useAuditLogExport();
  const [notice, setNotice] = useState<string | null>(null);

  const data = audit.data;
  const failure = audit.error ?? exported.error ?? null;
  const entries = data?.entries ?? [];
  const pageCount = Math.max(1, Math.ceil((data?.total ?? 0) / PAGE_SIZE));
  // A refetch can shrink the log; never strand the viewer on an empty page.
  const current = Math.min(page, pageCount - 1);

  const next = () => {
    const cursor = data?.next_cursor;
    if (!cursor) return;
    setCursors((stack) => {
      const grown = [...stack];
      grown[page + 1] = cursor;
      return grown;
    });
    setPage(page + 1);
  };

  const onExport = async () => {
    setNotice(null);
    const path = await save({
      title: t("audit.export.choose"),
      defaultPath: "boba-audit-log.csv",
      filters: [{ name: t("audit.export.filter"), extensions: ["csv"] }],
    });
    if (typeof path === "string") {
      exported.mutate(path, {
        onSuccess: (written) => setNotice(t("audit.export.done", { path: written })),
      });
    }
  };

  return (
    <section className="flex flex-col gap-4 p-6" aria-labelledby="settings-audit-title">
      <header className="flex flex-col gap-2">
        <h1 id="settings-audit-title" className="font-display text-2xl text-fg">
          {t("audit.title")}
        </h1>
        <p className="text-sm text-fg-muted">{t("audit.intro")}</p>
      </header>

      {audit.isPending ? <p className="text-sm text-fg-muted">{t("audit.loading")}</p> : null}
      {failure ? (
        <p role="alert" className="text-sm text-danger">
          {isAppError(failure) ? renderAppError(failure, tError) : t("audit.error")}
        </p>
      ) : null}

      {data ? (
        <>
          <p
            role="status"
            className={
              data.chain_intact
                ? "rounded-xl bg-surface-muted px-3 py-2 text-sm text-fg-muted"
                : "rounded-xl bg-surface-muted px-3 py-2 text-sm text-danger"
            }
          >
            {data.chain_intact
              ? t("audit.chain.ok", { rows: data.chain_rows })
              : t("audit.chain.broken", { seq: data.broken_at ?? "?" })}
          </p>

          <div className="flex flex-wrap items-center gap-3">
            <Button onClick={onExport} disabled={exported.isPending}>
              {t("audit.export")}
            </Button>
            {notice ? (
              <span role="status" className="text-sm text-fg-muted">
                {notice}
              </span>
            ) : null}
          </div>

          {entries.length === 0 ? (
            <p className="text-sm text-fg-muted">{t("audit.empty")}</p>
          ) : (
            <table className="w-full border-collapse text-left text-sm">
              <caption className="sr-only">{t("audit.title")}</caption>
              <thead>
                <tr className="border-b border-border text-xs text-fg-muted">
                  <th scope="col" className="py-2 pr-3 font-medium">
                    {t("audit.columns.time")}
                  </th>
                  <th scope="col" className="py-2 pr-3 font-medium">
                    {t("audit.columns.event")}
                  </th>
                  <th scope="col" className="py-2 pr-3 font-medium">
                    {t("audit.columns.target")}
                  </th>
                  <th scope="col" className="py-2 pr-3 font-medium">
                    {t("audit.columns.reason")}
                  </th>
                  <th scope="col" className="py-2 font-medium">
                    {t("audit.columns.package")}
                  </th>
                </tr>
              </thead>
              <tbody>
                {entries.map((entry) => (
                  <Row
                    key={entry.id}
                    entry={entry}
                    formatTime={(at) => formatTime(at, i18n.language)}
                    t={t}
                  />
                ))}
              </tbody>
            </table>
          )}

          {pageCount > 1 ? (
            <nav className="flex items-center gap-3" aria-label={t("audit.page.aria")}>
              <Button
                variant="ghost"
                onClick={() => setPage(Math.max(0, current - 1))}
                disabled={current === 0}
              >
                {t("audit.page.prev")}
              </Button>
              <span className="text-sm text-fg-muted">
                {t("audit.page.status", { page: current + 1, pages: pageCount })}
              </span>
              <Button variant="ghost" onClick={next} disabled={!data?.next_cursor}>
                {t("audit.page.next")}
              </Button>
            </nav>
          ) : null}
        </>
      ) : null}
    </section>
  );
}

type Translate = (key: string, options?: Record<string, unknown>) => string;

function Row({
  entry,
  formatTime,
  t,
}: {
  entry: AuditEntry;
  formatTime: (at: string) => string;
  t: Translate;
}) {
  const summary = entrySummary(entry);
  const event = eventLabel(t, entry.action);
  const outcome = entry.outcome ? outcomeLabel(t, entry.outcome) : null;

  return (
    <tr className="border-b border-border/60 align-top" data-kind={entry.kind}>
      <td className="whitespace-nowrap py-2 pr-3 text-fg-muted">
        <time dateTime={entry.at}>{formatTime(entry.at)}</time>
      </td>
      <td className="py-2 pr-3">
        <span className="mr-2 rounded-lg bg-surface-muted px-2 py-0.5 text-xs text-fg-muted">
          {kindLabel(t, entry.kind)}
        </span>
        <span className="text-fg">{event}</span>
        {outcome ? <span className="ml-2 text-xs text-fg-muted">({outcome})</span> : null}
      </td>
      <td className="py-2 pr-3 text-fg-muted">
        {entry.application_id ? (
          <a
            className="underline decoration-dotted hover:text-fg"
            href={`#/studio?app=${encodeURIComponent(entry.application_id)}`}
          >
            {t("audit.open_package")}
          </a>
        ) : (
          (entry.target_id ?? entry.target_kind)
        )}
      </td>
      <td className="py-2 pr-3 text-fg-muted">
        {summary ? <span className="block">{summary}</span> : null}
        {entry.detail_json && entry.detail_json !== "{}" ? (
          <details className="mt-1">
            <summary className="cursor-pointer text-xs text-fg-muted">{t("audit.details")}</summary>
            <code className="mt-1 block break-all text-xs">{entry.detail_json}</code>
          </details>
        ) : null}
      </td>
      <td className="py-2">
        {entry.payload_blake3 ? (
          <code className="block break-all text-xs text-fg-muted">{entry.payload_blake3}</code>
        ) : (
          <span className="text-fg-muted">—</span>
        )}
      </td>
    </tr>
  );
}

function formatTime(at: string, locale: string): string {
  const date = new Date(at);
  if (Number.isNaN(date.getTime())) return at;
  return new Intl.DateTimeFormat(locale, {
    dateStyle: "short",
    timeStyle: "medium",
  }).format(date);
}
