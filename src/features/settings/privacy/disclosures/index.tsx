import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { useDisclosure, useSaveDisclosure } from "@/features/today/needs-you/use-needs-you";
import type { ClassificationMode } from "@/lib/ipc/bindings";

/**
 * Settings → Privacy → Disclosures (S8.4, AC3, NFR-3): the same plain-language
 * disclosure Today shows the first time reply reading is used, plus the two
 * choices it offers — let the agent CLI read matched recruiter email, or keep
 * classification deterministic and on this device.
 *
 * The screen never sends anything: it only records the choice. The core refuses
 * the LLM mode while the disclosure has not been accepted.
 */
export default function SettingsPrivacyDisclosures() {
  const { t } = useTranslation("status_proposals");
  const disclosure = useDisclosure();
  const save = useSaveDisclosure();
  // The local pick while the user has not saved yet: the stored mode otherwise.
  const [mode, setMode] = useState<ClassificationMode | null>(null);
  const [autoConfirm, setAutoConfirm] = useState<boolean | null>(null);

  const stored = disclosure.data;
  const chosenMode = mode ?? stored?.mode ?? "deterministic";
  const chosenAutoConfirm = autoConfirm ?? stored?.auto_confirm ?? false;

  return (
    <section className="flex max-w-2xl flex-col gap-4 p-6" aria-labelledby="disclosures-title">
      <div>
        <h1 id="disclosures-title" className="font-display text-2xl text-fg">
          {t("settings.title")}
        </h1>
        <p className="text-sm text-fg-muted">{t("settings.subtitle")}</p>
      </div>

      <article className="flex flex-col gap-2 rounded-2xl border border-border bg-surface p-4">
        <h2 className="font-display text-lg text-fg">{t("disclosure.title")}</h2>
        <p className="text-sm text-fg-muted">{t("disclosure.body")}</p>
        <p className="text-sm text-fg-muted">{t("disclosure.hint")}</p>
      </article>

      <fieldset className="flex flex-col gap-2">
        <legend className="font-medium text-fg">{t("settings.mode")}</legend>
        <label className="flex items-start gap-2 text-sm text-fg">
          <input
            type="radio"
            name="classification-mode"
            checked={chosenMode === "llm"}
            onChange={() => setMode("llm")}
          />
          <span>{t("settings.mode_llm")}</span>
        </label>
        <label className="flex items-start gap-2 text-sm text-fg">
          <input
            type="radio"
            name="classification-mode"
            checked={chosenMode === "deterministic"}
            onChange={() => setMode("deterministic")}
          />
          <span>{t("settings.mode_deterministic")}</span>
        </label>
      </fieldset>

      <label className="flex items-start gap-2 text-sm text-fg">
        <input
          type="checkbox"
          checked={chosenAutoConfirm}
          onChange={(event) => setAutoConfirm(event.target.checked)}
        />
        <span className="flex flex-col">
          <span>{t("settings.auto_confirm")}</span>
          <span className="text-fg-muted">{t("settings.auto_confirm_hint")}</span>
        </span>
      </label>

      <div className="flex items-center gap-2">
        <Button
          type="button"
          disabled={save.isPending}
          onClick={() =>
            save.mutate(
              { mode: chosenMode, autoConfirm: chosenAutoConfirm, acknowledge: true },
              { onSuccess: () => setMode(null) },
            )
          }
        >
          {t("settings.save")}
        </Button>
        {save.isSuccess ? (
          <span className="text-sm text-fg-muted">{t("settings.saved")}</span>
        ) : null}
        {save.isError ? (
          <span role="alert" className="text-sm text-danger">
            {t("error.action")}
          </span>
        ) : null}
      </div>

      {stored?.acknowledged ? (
        <p className="text-xs text-fg-subtle">{t("settings.acknowledged")}</p>
      ) : null}
    </section>
  );
}
