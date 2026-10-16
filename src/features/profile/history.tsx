import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { useFactHistory, useRestoreFact } from "./use-profile";

/**
 * Version history for one fact (AC3, EXPERIENCE §9.10 "History"). Facts are
 * immutable, so restoring is another append: the panel keeps the full list.
 */
export function FactHistory({
  itemId,
  label,
  onClose,
}: {
  itemId: string;
  label: string;
  onClose: () => void;
}) {
  const { t } = useTranslation("profile");
  const { t: tError } = useTranslation();
  const history = useFactHistory(itemId);
  const restore = useRestoreFact(itemId);

  return (
    <section
      aria-label={t("history")}
      className="flex flex-col gap-3 rounded-2xl border border-border bg-surface p-4"
    >
      <div className="flex items-center justify-between gap-2">
        <p className="font-display text-lg text-fg">
          {t("history")} · {label}
        </p>
        <Button variant="ghost" size="sm" onClick={onClose}>
          {t("history.close")}
        </Button>
      </div>

      {history.isPending ? <p className="text-sm text-fg-muted">{t("history.loading")}</p> : null}
      {history.isError ? (
        <p className="text-sm text-danger">
          {isAppError(history.error)
            ? renderAppError(history.error, tError)
            : tError("errors.unknown")}
        </p>
      ) : null}
      {history.data && history.data.length < 2 ? (
        <p className="text-sm text-fg-muted">{t("history.empty")}</p>
      ) : null}

      <ol className="flex flex-col gap-2">
        {history.data?.map((revision, index) => (
          <li
            key={revision.revision_id}
            className="flex flex-col gap-1 rounded-xl bg-surface-muted p-3"
          >
            <p className="text-xs text-fg-subtle">
              {t("history.version", { version: revision.profile_version })}
              {index === 0 ? ` · ${t("history.current")}` : ""}
            </p>
            <p className="text-sm text-fg">{revision.text_en ?? revision.text_es}</p>
            {/* The revision does not exist in this language yet: say so rather
                than showing a blank line. */}
            {revision.text_en === null || revision.text_es === null ? (
              <p className="text-xs text-warning">
                {t(`profile:fact.missing.${revision.missing_variant ?? "en"}`)}
              </p>
            ) : null}
            {index === 0 ? null : (
              <Button
                variant="outline"
                size="sm"
                disabled={restore.isPending}
                onClick={() => restore.mutate(revision.revision_id)}
              >
                {t("history.restore")}
              </Button>
            )}
          </li>
        ))}
      </ol>

      {restore.isError ? (
        <p className="text-sm text-danger">
          {isAppError(restore.error)
            ? renderAppError(restore.error, tError)
            : tError("errors.unknown")}
        </p>
      ) : null}
    </section>
  );
}
