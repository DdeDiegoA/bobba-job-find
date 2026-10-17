import { useTranslation } from "react-i18next";
import { formatDateLabel, formatNumber } from "@/lib/format";
import { currentLocale } from "@/lib/i18n";
import type { Purpose } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import {
  useFxSettings,
  useNetworkActivity,
  useSetFx,
  useSetUpdateChecks,
  useUpdateSettings,
} from "./use-network";

/** Every purpose's heading; a static map, so the i18n gate can see each key. */
const PURPOSE_LABELS: Record<Purpose, string> = {
  sources: "network:purpose.sources",
  email: "network:purpose.email",
  agent_cli: "network:purpose.agent_cli",
  update: "network:purpose.update",
  model_download: "network:purpose.model_download",
  fx: "network:purpose.fx",
};

/**
 * Settings → Privacy → Network activity (S10.4, NFR-1, AC1) and the FX opt-in
 * that decides whether a salary ever shows a converted amount (FR-115, AC2).
 *
 * The list is read-only: it shows every destination Boba contacted or is
 * configured to contact, grouped by purpose, with the host and the time only.
 * There is no telemetry group to leave out — the core's purpose list has no such
 * variant.
 */
export default function SettingsPrivacyNetwork() {
  const { t } = useTranslation("network");
  const { t: tError } = useTranslation();
  const locale = currentLocale();
  const activity = useNetworkActivity();
  const fx = useFxSettings();
  const setFx = useSetFx();
  const update = useUpdateSettings();
  const setUpdate = useSetUpdateChecks();

  const data = activity.data;
  const failure =
    activity.error ?? fx.error ?? setFx.error ?? update.error ?? setUpdate.error ?? null;
  const fxData = fx.data;
  const fxOn = fxData?.enabled ?? false;
  const updateOn = update.data?.enabled ?? false;
  const updateFailed = Boolean(setUpdate.error);

  return (
    <section className="flex flex-col gap-4 p-6" aria-labelledby="settings-network-title">
      <h1 id="settings-network-title" className="font-display text-2xl text-fg">
        {t("title")}
      </h1>
      <p className="text-sm text-fg-muted">{t("intro")}</p>

      {failure ? (
        <p role="alert" className="text-sm text-danger">
          {isAppError(failure)
            ? renderAppError(failure, tError)
            : updateFailed
              ? t("update.error")
              : t("fx.error")}
        </p>
      ) : null}

      {data && data.groups.length === 0 ? (
        <p className="text-sm text-fg-muted">{t("empty")}</p>
      ) : null}

      {data?.groups.map((group) => (
        <section
          key={group.purpose}
          className="rounded-2xl border border-border bg-surface p-4"
          aria-labelledby={`network-${group.purpose}`}
          data-purpose={group.purpose}
        >
          <h2 id={`network-${group.purpose}`} className="font-display text-lg text-fg">
            {t(PURPOSE_LABELS[group.purpose])}
          </h2>
          <table className="mt-2 w-full text-left text-sm">
            <thead className="text-xs text-fg-muted">
              <tr>
                <th scope="col">{t("columns.destination")}</th>
                <th scope="col">{t("columns.last_seen")}</th>
                <th scope="col">{t("columns.requests")}</th>
              </tr>
            </thead>
            <tbody>
              {group.destinations.map((destination) => (
                <tr key={`${group.purpose}:${destination.name}`}>
                  <td className="py-1">
                    <span className="font-mono text-fg">{destination.name}</span>
                    {destination.note ? (
                      <span className="ml-2 text-xs text-fg-muted">{destination.note}</span>
                    ) : null}
                  </td>
                  <td className="py-1 text-fg-muted">
                    {destination.last_seen
                      ? formatDateLabel(destination.last_seen, locale)
                      : t("never")}
                  </td>
                  <td className="py-1 text-fg-muted">
                    {destination.requests > 0
                      ? t("requests", { count: formatNumber(destination.requests, locale) })
                      : "—"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      ))}

      <p className="rounded-xl bg-surface-muted px-3 py-2 text-sm text-fg-muted">
        {t("no_telemetry")}
      </p>

      <section
        className="rounded-2xl border border-border bg-surface p-4"
        aria-labelledby="settings-network-update"
      >
        <h2 id="settings-network-update" className="font-display text-lg text-fg">
          {t("update.title")}
        </h2>
        <p className="mt-1 text-sm text-fg-muted">{t("update.description")}</p>
        <label className="mt-3 flex items-center gap-2 text-sm text-fg">
          <input
            type="checkbox"
            checked={updateOn}
            disabled={update.isPending || setUpdate.isPending}
            onChange={(event) => setUpdate.mutate(event.currentTarget.checked)}
          />
          {t("update.toggle")}
        </label>
        <p className="mt-2 text-sm text-fg-muted">{updateOn ? t("update.on") : t("update.off")}</p>
      </section>

      <section
        className="rounded-2xl border border-border bg-surface p-4"
        aria-labelledby="settings-network-fx"
      >
        <h2 id="settings-network-fx" className="font-display text-lg text-fg">
          {t("fx.title")}
        </h2>
        <p className="mt-1 text-sm text-fg-muted">{t("fx.description")}</p>
        <label className="mt-3 flex items-center gap-2 text-sm text-fg">
          <input
            type="checkbox"
            checked={fxOn}
            disabled={fx.isPending || setFx.isPending}
            onChange={(event) => setFx.mutate(event.currentTarget.checked)}
          />
          {t("fx.toggle")}
        </label>
        <p className="mt-2 text-sm text-fg-muted">{fxOn ? t("fx.on") : t("fx.off")}</p>
        {fxOn && fxData ? (
          <p className="mt-1 text-sm text-fg-muted">
            {fxData.as_of
              ? t("fx.rate_as_of", {
                  date: formatDateLabel(fxData.as_of, locale),
                  currency: fxData.target,
                })
              : t("fx.no_rate")}
          </p>
        ) : null}
      </section>
    </section>
  );
}
