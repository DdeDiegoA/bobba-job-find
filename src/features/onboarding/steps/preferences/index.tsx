import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Preferences } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import type { StepProps } from "../../wizard/machine";
import { PreferencesForm } from "./prefs-form";
import { usePreferences, useSavePreferences } from "./use-preferences";

/**
 * Step — what I am looking for (FR-7). Every edit is saved as it happens, so
 * Back, a quit mid-step and the Settings page all see the same document; the
 * core turns each change into a new prefs version (AC2).
 */
export default function PreferencesStep(_props: StepProps) {
  const { t } = useTranslation();
  const { t: tError } = useTranslation();
  const preferences = usePreferences();
  const save = useSavePreferences();
  const [draft, setDraft] = useState<Preferences | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  const values = draft ?? preferences.data?.values;
  const change = (next: Preferences) => {
    setDraft(next);
    setSaveError(null);
    save.mutate(next, {
      onError: (error) =>
        setSaveError(
          isAppError(error) ? renderAppError(error, tError) : t("preferences:error.save"),
        ),
    });
  };

  return (
    <div className="flex flex-col gap-6">
      <p className="text-fg-muted">{t("preferences:step.body")}</p>

      {preferences.isPending ? (
        <p className="text-sm text-fg-muted">{t("preferences:error.loading")}</p>
      ) : null}
      {preferences.isError ? (
        <p className="text-sm text-danger">
          {isAppError(preferences.error)
            ? renderAppError(preferences.error, tError)
            : t("preferences:error.load")}
        </p>
      ) : null}
      {saveError ? <p className="text-sm text-danger">{saveError}</p> : null}

      {values ? <PreferencesForm values={values} onChange={change} /> : null}
    </div>
  );
}
