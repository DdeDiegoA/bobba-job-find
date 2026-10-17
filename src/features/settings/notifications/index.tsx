import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { NotificationSettings, QuietHours } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { useNotifications, useSaveNotifications } from "./use-notifications";

const HOURS = Array.from({ length: 24 }, (_, hour) => hour);
const hourText = (hour: number) => `${String(hour).padStart(2, "0")}:00`;
const MAX_TOP_MATCHES = 10;

const checkboxClass = "size-4 rounded border-border";
const selectClass = "rounded-xl border border-border bg-surface-muted px-3 py-2 text-fg";
const numberClass = "w-24 rounded-xl border border-border bg-surface-muted px-3 py-2 text-fg";

/** One labelled checkbox: the label is the control's accessible name. */
function Check({
  id,
  label,
  checked,
  onChange,
}: {
  id: string;
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label htmlFor={id} className="flex items-center gap-2 text-sm text-fg">
      <input
        id={id}
        type="checkbox"
        className={checkboxClass}
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      {label}
    </label>
  );
}

function HourSelect({
  id,
  label,
  value,
  onChange,
}: {
  id: string;
  label: string;
  value: number;
  onChange: (hour: number) => void;
}) {
  return (
    <label htmlFor={id} className="flex items-center gap-2 text-sm text-fg-muted">
      {label}
      <select
        id={id}
        className={selectClass}
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
      >
        {HOURS.map((hour) => (
          <option key={hour} value={hour}>
            {hourText(hour)}
          </option>
        ))}
      </select>
    </label>
  );
}

/**
 * Settings → Notifications (S5.3, FR-107): which events notify, how many top
 * matches the digest names, and the quiet hours. Save is one explicit step, so
 * toggling a box never changes what tonight's run does by accident.
 */
export default function SettingsNotifications() {
  const { t } = useTranslation("notifications");
  const { t: tError } = useTranslation();
  const query = useNotifications();
  const save = useSaveNotifications();
  const [draft, setDraft] = useState<NotificationSettings | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  const stored = query.data;
  const values = draft ?? stored;
  const dirty = Boolean(values && stored && JSON.stringify(values) !== JSON.stringify(stored));

  const edit = (change: (next: NotificationSettings) => void) => {
    if (!values) return;
    const next = structuredClone(values);
    change(next);
    setDraft(next);
    setSaveError(null);
  };

  const editQuiet = (change: (quiet: QuietHours) => void) =>
    edit((next) => change(next.quiet_hours));

  return (
    <section className="flex flex-col gap-4 p-6" aria-labelledby="settings-notifications-title">
      <h1 id="settings-notifications-title" className="font-display text-2xl text-fg">
        {t("title")}
      </h1>
      <p className="text-sm text-fg-muted">{t("intro")}</p>

      {query.isPending ? <p className="text-sm text-fg-muted">{t("title")}</p> : null}
      {query.isError ? (
        <p className="text-sm text-danger">
          {isAppError(query.error) ? renderAppError(query.error, tError) : t("error.load")}
        </p>
      ) : null}
      {saveError ? <p className="text-sm text-danger">{saveError}</p> : null}

      {values ? (
        <div className="flex flex-col gap-6">
          <fieldset className="flex flex-col gap-2">
            <legend className="mb-1 font-medium text-fg">{t("events.title")}</legend>
            <Check
              id="notify-digest"
              label={t("events.digest")}
              checked={values.events.digest}
              onChange={(checked) => edit((next) => (next.events.digest = checked))}
            />
            <Check
              id="notify-outbox"
              label={t("events.outbox")}
              checked={values.events.outbox}
              onChange={(checked) => edit((next) => (next.events.outbox = checked))}
            />
            <Check
              id="notify-needs-you"
              label={t("events.needs_you")}
              checked={values.events.needs_you}
              onChange={(checked) => edit((next) => (next.events.needs_you = checked))}
            />
            <Check
              id="notify-errors"
              label={t("events.errors")}
              checked={values.events.errors}
              onChange={(checked) => edit((next) => (next.events.errors = checked))}
            />
            <p className="text-xs text-fg-muted">{t("security.hint")}</p>
          </fieldset>

          <div className="flex flex-col gap-1">
            <label htmlFor="notify-cap" className="text-sm text-fg">
              {t("digest_cap.label")}
            </label>
            <input
              id="notify-cap"
              type="number"
              min={0}
              max={MAX_TOP_MATCHES}
              className={numberClass}
              value={values.top_matches_shown}
              onChange={(event) =>
                edit(
                  (next) =>
                    (next.top_matches_shown = Math.max(
                      0,
                      Math.min(MAX_TOP_MATCHES, Number(event.target.value) || 0),
                    )),
                )
              }
            />
            <p className="text-xs text-fg-muted">{t("digest_cap.hint")}</p>
          </div>

          <Check
            id="notify-empty"
            label={t("empty.label")}
            checked={values.notify_when_empty}
            onChange={(checked) => edit((next) => (next.notify_when_empty = checked))}
          />

          <fieldset className="flex flex-col gap-2">
            <legend className="mb-1 font-medium text-fg">{t("quiet.title")}</legend>
            <Check
              id="quiet-enabled"
              label={t("quiet.enabled")}
              checked={values.quiet_hours.enabled}
              onChange={(checked) => editQuiet((quiet) => (quiet.enabled = checked))}
            />
            {values.quiet_hours.enabled ? (
              <>
                <div className="flex flex-wrap items-center gap-4">
                  <HourSelect
                    id="quiet-from"
                    label={t("quiet.from")}
                    value={values.quiet_hours.from}
                    onChange={(hour) => editQuiet((quiet) => (quiet.from = hour))}
                  />
                  <HourSelect
                    id="quiet-to"
                    label={t("quiet.to")}
                    value={values.quiet_hours.to}
                    onChange={(hour) => editQuiet((quiet) => (quiet.to = hour))}
                  />
                </div>
                <Check
                  id="quiet-notify-during"
                  label={t("quiet.notify_during")}
                  checked={values.quiet_hours.notify_during}
                  onChange={(checked) => editQuiet((quiet) => (quiet.notify_during = checked))}
                />
              </>
            ) : null}
            <p className="text-xs text-fg-muted">{t("quiet.hint")}</p>
          </fieldset>

          <div className="flex items-center gap-3">
            <Button
              type="button"
              disabled={!dirty || save.isPending}
              onClick={() => {
                if (!values) return;
                setSaveError(null);
                save.mutate(values, {
                  onSuccess: () => setDraft(null),
                  onError: (error) =>
                    setSaveError(
                      isAppError(error) ? renderAppError(error, tError) : t("error.save"),
                    ),
                });
              }}
            >
              {t("save")}
            </Button>
            {save.isSuccess && !dirty ? (
              <span role="status" className="text-sm text-fg-muted">
                {t("saved")}
              </span>
            ) : null}
          </div>
        </div>
      ) : null}
    </section>
  );
}
