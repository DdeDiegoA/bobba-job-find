import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { useExportData } from "./use-export";

/**
 * Settings → Privacy → Export (S10.1, FR-110).
 *
 * The export is unencrypted by design — it is the user's way out of the app —
 * so the warning comes first: the page states what leaves the app and keeps
 * the button disabled until the user acknowledges it (AC1). The folder picker
 * is the only thing the page decides; every format and every file is the
 * core's.
 */
export default function SettingsPrivacyExport() {
  const { t } = useTranslation("export");
  const { t: tError } = useTranslation();

  const [destination, setDestination] = useState<string | null>(null);
  const [acknowledged, setAcknowledged] = useState(false);
  const run = useExportData();

  const choose = async () => {
    const path = await open({
      title: t("choose"),
      directory: true,
      multiple: false,
    });
    if (typeof path === "string") setDestination(path);
  };

  const errorText = run.error
    ? isAppError(run.error)
      ? renderAppError(run.error, tError)
      : t("error.action")
    : null;

  return (
    <section className="flex flex-col gap-6 p-6" aria-labelledby="settings-export-title">
      <div className="flex flex-col gap-2">
        <h1 id="settings-export-title" className="font-display text-2xl text-fg">
          {t("title")}
        </h1>
        <p className="text-sm text-fg-muted">{t("intro")}</p>
      </div>

      {/* AC1: the unencrypted warning comes before the export can run. */}
      <section
        className="flex flex-col gap-3 rounded-2xl border border-warning/40 p-4"
        aria-labelledby="export-warning-title"
      >
        <h2 id="export-warning-title" className="font-display text-lg text-fg">
          {t("warning.title")}
        </h2>
        <p className="text-sm text-fg-muted">{t("warning.body")}</p>
        <label className="flex items-start gap-2 text-sm text-fg-muted">
          <input
            type="checkbox"
            checked={acknowledged}
            onChange={(event) => setAcknowledged(event.target.checked)}
          />
          {t("warning.ack")}
        </label>
      </section>

      <section
        className="flex flex-col gap-3 rounded-2xl border border-border p-4"
        aria-labelledby="export-run-title"
      >
        <h2 id="export-run-title" className="font-display text-lg text-fg">
          {t("action")}
        </h2>

        <div className="flex flex-wrap items-center gap-2">
          <Button type="button" variant="outline" size="sm" onClick={choose}>
            {t("choose")}
          </Button>
          <span className="break-all text-xs text-fg-muted">{destination ?? t("no_folder")}</span>
        </div>

        <Button
          type="button"
          disabled={!destination || !acknowledged || run.isPending}
          onClick={() => destination && run.mutate(destination)}
        >
          {t("action")}
        </Button>

        {errorText ? (
          <p role="alert" className="text-sm text-danger">
            {errorText}
          </p>
        ) : null}

        {run.isSuccess ? (
          <p role="status" className="text-sm text-fg-muted">
            {t("done", {
              files: run.data.files,
              destination: run.data.destination,
            })}
          </p>
        ) : null}
      </section>
    </section>
  );
}
