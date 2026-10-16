import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { Preferences } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { preferenceValuesEqual } from "../../onboarding/steps/preferences/prefs";
import { PreferencesForm } from "../../onboarding/steps/preferences/prefs-form";
import {
  usePreferences,
  useSavePreferences,
} from "../../onboarding/steps/preferences/use-preferences";

/**
 * Settings → Preferences (FR-7): every value captured in onboarding is edited
 * here later. Save goes through the core, which appends a new prefs version and
 * marks the open scores stale (AC2, FR-14); it stays disabled while nothing
 * changed, so browsing never creates a version.
 */
export default function SettingsPreferences() {
  const { t } = useTranslation("preferences");
  const { t: tError } = useTranslation();
  const preferences = usePreferences();
  const save = useSavePreferences();
  const [draft, setDraft] = useState<Preferences | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  const stored = preferences.data;
  const values = draft ?? stored?.values;
  const dirty = Boolean(values && stored && !preferenceValuesEqual(values, stored.values));

  return (
    <section className="flex flex-col gap-4 p-6" aria-labelledby="settings-preferences-title">
      <div className="flex items-center gap-2">
        <h1 id="settings-preferences-title" className="font-display text-2xl text-fg">
          {t("title")}
        </h1>
        {stored ? (
          <span
            className="rounded-xl bg-surface-muted px-2 py-1 text-xs text-fg-muted"
            title={t("version.hint")}
          >
            {t("version", { version: stored.version })}
          </span>
        ) : null}
      </div>

      {/* FR-14: a change left the stored scores stale until the next re-score. */}
      {stored?.scores_stale ? (
        <p role="status" className="rounded-xl bg-surface-muted px-3 py-2 text-sm text-fg-muted">
          {t("stale")}
        </p>
      ) : null}

      {preferences.isPending ? <p className="text-sm text-fg-muted">{t("error.loading")}</p> : null}
      {preferences.isError ? (
        <p className="text-sm text-danger">
          {isAppError(preferences.error)
            ? renderAppError(preferences.error, tError)
            : t("error.load")}
        </p>
      ) : null}
      {saveError ? <p className="text-sm text-danger">{saveError}</p> : null}

      {values ? <PreferencesForm values={values} onChange={setDraft} /> : null}

      <div>
        <Button
          type="button"
          disabled={!dirty || save.isPending}
          onClick={() => {
            if (!values) return;
            setSaveError(null);
            save.mutate(values, {
              onSuccess: () => setDraft(null),
              onError: (error) =>
                setSaveError(isAppError(error) ? renderAppError(error, tError) : t("error.save")),
            });
          }}
        >
          {t("save")}
        </Button>
      </div>
    </section>
  );
}
