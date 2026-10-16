import { useTranslation } from "react-i18next";
import type { ApplicationDetail } from "@/lib/ipc/bindings";

/**
 * The card detail (AC3, FR-94): the Offer snapshot, the Package the
 * application carries, the timeline of Stage changes, the next action and the
 * notes. Read-only — the only write in this slice is a Stage move.
 */
export function Detail({
  detail,
  loading,
  onClose,
}: {
  detail: ApplicationDetail | undefined;
  loading: boolean;
  onClose: () => void;
}) {
  const { t } = useTranslation("tracker");

  return (
    <aside
      aria-label={t("detail.title")}
      className="flex w-80 shrink-0 flex-col gap-4 overflow-y-auto rounded-2xl border border-border bg-surface p-4"
    >
      <div className="flex items-start justify-between gap-2">
        <h2 className="font-display text-lg text-fg">{t("detail.title")}</h2>
        <button type="button" className="text-xs text-fg-muted underline" onClick={onClose}>
          {t("detail.close")}
        </button>
      </div>

      {loading || !detail ? (
        <p className="text-sm text-fg-muted">{t("loading")}</p>
      ) : (
        <>
          <div className="flex flex-col gap-1">
            <p className="text-sm text-fg">{detail.title}</p>
            <p className="text-xs text-fg-subtle">{detail.company}</p>
            <p className="text-xs text-fg-muted">{t(`tracker:stage.${detail.stage}`)}</p>
          </div>

          <section className="flex flex-col gap-1">
            <h3 className="text-xs uppercase text-fg-subtle">{t("detail.snapshot")}</h3>
            <p className="text-sm text-fg">{detail.title}</p>
            <p className="text-xs text-fg-subtle">{detail.company}</p>
            {detail.lang ? (
              <p className="text-xs text-fg-muted">{t("detail.lang", { lang: detail.lang })}</p>
            ) : null}
            {detail.deadline ? (
              <p className="text-xs text-fg-muted">
                {t("detail.deadline", { date: detail.deadline })}
              </p>
            ) : null}
            {detail.apply_url ? (
              <a
                className="text-xs text-primary underline"
                href={detail.apply_url}
                rel="noreferrer"
                target="_blank"
              >
                {t("detail.open")}
              </a>
            ) : null}
          </section>

          <section className="flex flex-col gap-1">
            <h3 className="text-xs uppercase text-fg-subtle">{t("detail.package")}</h3>
            {detail.package.length === 0 ? (
              <p className="text-xs text-fg-muted">{t("detail.package.empty")}</p>
            ) : (
              <ul className="flex flex-col gap-1">
                {detail.package.map((item) => (
                  <li key={item.document_id} className="text-xs text-fg">
                    {t("detail.package.item", {
                      kind: t(`tracker:kind.${item.kind}`),
                      lang: item.lang,
                      version: item.version,
                    })}{" "}
                    <span className="text-fg-subtle">
                      {item.verified
                        ? t("detail.package.verified")
                        : t("detail.package.unverified")}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </section>

          <section className="flex flex-col gap-1">
            <h3 className="text-xs uppercase text-fg-subtle">{t("detail.timeline")}</h3>
            {detail.timeline.length === 0 ? (
              <p className="text-xs text-fg-muted">{t("detail.timeline.empty")}</p>
            ) : (
              <ol className="flex flex-col gap-1">
                {detail.timeline.map((event) => (
                  <li key={event.id} className="text-xs text-fg">
                    {event.from_stage
                      ? t("detail.timeline.move", {
                          from: t(`tracker:stage.${event.from_stage}`),
                          to: t(`tracker:stage.${event.to_stage}`),
                        })
                      : t("detail.timeline.created")}{" "}
                    <span className="text-fg-subtle">
                      {t(`tracker:detail.source.${event.source}`)} · {event.created_at}
                    </span>
                  </li>
                ))}
              </ol>
            )}
          </section>

          <section className="flex flex-col gap-1">
            <h3 className="text-xs uppercase text-fg-subtle">{t("detail.nextAction")}</h3>
            <p className="text-sm text-fg">{detail.next_action ?? t("detail.none")}</p>
          </section>

          <section className="flex flex-col gap-1">
            <h3 className="text-xs uppercase text-fg-subtle">{t("detail.notes")}</h3>
            <p className="text-sm text-fg">{detail.notes ?? t("detail.none")}</p>
          </section>
        </>
      )}
    </aside>
  );
}
