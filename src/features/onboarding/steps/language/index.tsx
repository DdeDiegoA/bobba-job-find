import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { type Theme, useTheme } from "@/app/theme";
import { currentLocale, setLocale } from "@/lib/i18n";
import { type Locale, SUPPORTED_LOCALES } from "@/lib/i18n/detect";
import type { StepProps } from "../../wizard/machine";

const THEMES: Theme[] = ["system", "light", "dark"];

/**
 * Step 1 — language & theme (FR-1). The choice applies live, so every later
 * step and screen is already in the chosen language (PRD §4.1). It lands in the
 * same stores the shell toggle uses, so it also survives the wizard (AC2).
 */
export default function LanguageStep({ answers, setAnswer }: StepProps) {
  const { t } = useTranslation();
  const { theme, setTheme } = useTheme();
  const locale = (answers.locale as Locale | undefined) ?? currentLocale();
  const mode = (answers.theme as Theme | undefined) ?? theme;

  useEffect(() => {
    setLocale(locale);
  }, [locale]);

  useEffect(() => {
    setTheme(mode);
  }, [mode, setTheme]);

  return (
    <div className="flex flex-col gap-6">
      <p className="text-fg-muted">{t("onboarding:steps.language.body")}</p>

      <fieldset className="flex flex-col gap-2">
        <legend className="mb-1 font-medium text-fg">
          {t("onboarding:steps.language.language")}
        </legend>
        <div className="flex flex-wrap gap-2">
          {SUPPORTED_LOCALES.map((lng) => (
            <label
              key={lng}
              className="flex cursor-pointer items-center gap-2 rounded-xl border border-border bg-surface px-4 py-2 text-fg has-checked:border-primary has-checked:bg-primary-soft"
            >
              <input
                type="radio"
                name="onboarding-locale"
                checked={locale === lng}
                onChange={() => setAnswer("locale", lng)}
              />
              {t(`onboarding:steps.language.locale.${lng}`)}
            </label>
          ))}
        </div>
      </fieldset>

      <fieldset className="flex flex-col gap-2">
        <legend className="mb-1 font-medium text-fg">{t("onboarding:steps.language.theme")}</legend>
        <div className="flex flex-wrap gap-2">
          {THEMES.map((option) => (
            <label
              key={option}
              className="flex cursor-pointer items-center gap-2 rounded-xl border border-border bg-surface px-4 py-2 text-fg has-checked:border-primary has-checked:bg-primary-soft"
            >
              <input
                type="radio"
                name="onboarding-theme"
                checked={mode === option}
                onChange={() => setAnswer("theme", option)}
              />
              {t(`onboarding:steps.language.themeOption.${option}`)}
            </label>
          ))}
        </div>
      </fieldset>
    </div>
  );
}
