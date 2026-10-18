import { useTranslation } from "react-i18next";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { useAutostart, useSetAutostart } from "./use-autostart";

/**
 * Settings → Notifications → Launch at login (S5.4, FR-44). The switch applies
 * immediately (it is the OS setting, not a notification preference), and every
 * change re-reads `autostartStatus` so the UI reports what the OS actually did.
 */
export default function LaunchAtLogin() {
  const { t } = useTranslation("notifications");
  const { t: tError } = useTranslation();
  const query = useAutostart();
  const set = useSetAutostart();
  const value = query.data;
  const error = set.error;

  return (
    <fieldset className="flex flex-col gap-2">
      <legend className="mb-1 font-medium text-fg">{t("autostart.title")}</legend>

      {query.isPending ? <p className="text-sm text-fg-muted">{t("autostart.loading")}</p> : null}
      {query.isError ? (
        <p className="text-sm text-danger">
          {isAppError(query.error)
            ? renderAppError(query.error, tError)
            : t("autostart.error.load")}
        </p>
      ) : null}

      {value ? (
        <label htmlFor="launch-at-login" className="flex items-center gap-2 text-sm text-fg">
          <input
            id="launch-at-login"
            type="checkbox"
            className="size-4 rounded border-border"
            checked={value.enabled}
            disabled={set.isPending}
            onChange={(event) => set.mutate(event.target.checked)}
          />
          {t("autostart.label")}
        </label>
      ) : null}

      <p className="text-xs text-fg-muted">{t("autostart.hint")}</p>

      {error ? (
        <p role="alert" className="text-sm text-danger">
          {isAppError(error) ? renderAppError(error, tError) : t("autostart.error")}
        </p>
      ) : null}
    </fieldset>
  );
}
