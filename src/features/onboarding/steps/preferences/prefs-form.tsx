import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { Preferences, SalaryFloor } from "@/lib/ipc/bindings";
import {
  CURRENCY_OPTIONS,
  HOURS,
  MONEY_PERIODS,
  SALARY_FIRMNESS_OPTIONS,
  SENIORITY_OPTIONS,
  salaryFloorFromInput,
  WORK_MODE_OPTIONS,
  WORKING_LANGUAGE_OPTIONS,
} from "./prefs";

const selectClass = "rounded-xl border border-border bg-surface-muted px-3 py-2 text-fg";
const textClass = "rounded-xl border border-border bg-surface-muted px-3 py-2 text-fg";

/** Text of the amount field; the stored floor is in minor units (AD-17). */
const amountText = (floor: SalaryFloor | null): string =>
  floor ? String(floor.amount_minor / 100) : "";

/**
 * The FR-7 fields, shared by the onboarding step and Settings → Preferences so
 * both ask exactly the same things. Controlled: the host decides when a change
 * becomes a new prefs version (AC2).
 */
export function PreferencesForm({
  values,
  onChange,
}: {
  values: Preferences;
  onChange: (next: Preferences) => void;
}) {
  const { t } = useTranslation("preferences");
  // Money is committed when I leave the amount field, so typing 60000 does not
  // create five versions; the other money choices are decisions on their own.
  const [money, setMoney] = useState(() => ({
    amount: amountText(values.salary_floor),
    currency: values.salary_floor?.currency ?? "USD",
    period: values.salary_floor?.period ?? ("year" as const),
    firmness: values.salary_floor?.firmness ?? ("hard" as const),
  }));
  const commitMoney = (next: typeof money) => {
    setMoney(next);
    onChange({
      ...values,
      salary_floor: salaryFloorFromInput(next.amount, next.currency, next.period, next.firmness),
    });
  };

  const set = <K extends keyof Preferences>(key: K, value: Preferences[K]) =>
    onChange({ ...values, [key]: value });

  return (
    <div className="flex flex-col gap-6">
      <ChipField
        id="pref-roles"
        label={t("roles")}
        placeholder={t("roles.placeholder")}
        items={values.roles}
        onChange={(roles) => set("roles", roles)}
      />

      <fieldset className="flex flex-col gap-2">
        <legend className="mb-1 font-medium text-fg">{t("seniority")}</legend>
        <div className="flex flex-wrap gap-4">
          <label className="flex flex-col gap-1 text-sm text-fg-muted" htmlFor="pref-seniority-min">
            {t("seniority.min")}
            <select
              id="pref-seniority-min"
              className={selectClass}
              value={values.seniority_min ?? ""}
              onChange={(event) =>
                set("seniority_min", (event.target.value || null) as Preferences["seniority_min"])
              }
            >
              <option value="">{t("seniority.any")}</option>
              {SENIORITY_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {t(option.labelKey)}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1 text-sm text-fg-muted" htmlFor="pref-seniority-max">
            {t("seniority.max")}
            <select
              id="pref-seniority-max"
              className={selectClass}
              value={values.seniority_max ?? ""}
              onChange={(event) =>
                set("seniority_max", (event.target.value || null) as Preferences["seniority_max"])
              }
            >
              <option value="">{t("seniority.any")}</option>
              {SENIORITY_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {t(option.labelKey)}
                </option>
              ))}
            </select>
          </label>
        </div>
      </fieldset>

      <fieldset className="flex flex-col gap-2">
        <legend className="mb-1 font-medium text-fg">{t("salary")}</legend>
        <div className="flex flex-wrap items-end gap-4">
          <label className="flex flex-col gap-1 text-sm text-fg-muted" htmlFor="pref-salary-amount">
            {t("salary.amount")}
            <input
              id="pref-salary-amount"
              className={textClass}
              inputMode="decimal"
              value={money.amount}
              onChange={(event) => setMoney({ ...money, amount: event.target.value })}
              onBlur={() => commitMoney(money)}
            />
          </label>
          <label
            className="flex flex-col gap-1 text-sm text-fg-muted"
            htmlFor="pref-salary-currency"
          >
            {t("salary.currency")}
            <select
              id="pref-salary-currency"
              className={selectClass}
              value={money.currency}
              onChange={(event) => commitMoney({ ...money, currency: event.target.value })}
            >
              {CURRENCY_OPTIONS.map((currency) => (
                <option key={currency} value={currency}>
                  {currency}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1 text-sm text-fg-muted" htmlFor="pref-salary-period">
            {t("salary.period")}
            <select
              id="pref-salary-period"
              className={selectClass}
              value={money.period}
              onChange={(event) =>
                commitMoney({ ...money, period: event.target.value as typeof money.period })
              }
            >
              {MONEY_PERIODS.map((period) => (
                <option key={period} value={period}>
                  {t(`preferences:money.period.${period}`)}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="flex flex-wrap gap-4">
          {SALARY_FIRMNESS_OPTIONS.map((option) => (
            <label
              key={option.value}
              className="flex cursor-pointer items-center gap-2 rounded-xl border border-border bg-surface px-4 py-2 text-sm text-fg has-checked:border-primary has-checked:bg-primary-soft"
            >
              <input
                type="radio"
                name="pref-salary-firmness"
                checked={money.firmness === option.value}
                onChange={() => commitMoney({ ...money, firmness: option.value })}
              />
              {t(option.labelKey)}
            </label>
          ))}
        </div>
      </fieldset>

      <ChipField
        id="pref-locations"
        label={t("locations")}
        placeholder={t("locations.placeholder")}
        items={values.locations}
        onChange={(locations) => set("locations", locations)}
      />

      <fieldset className="flex flex-col gap-2">
        <legend className="mb-1 font-medium text-fg">{t("timezone")}</legend>
        <div className="flex flex-wrap gap-4">
          {(["from", "to"] as const).map((edge) => (
            <label
              key={edge}
              className="flex flex-col gap-1 text-sm text-fg-muted"
              htmlFor={`pref-timezone-${edge}`}
            >
              {t(`preferences:timezone.${edge}`)}
              <select
                id={`pref-timezone-${edge}`}
                className={selectClass}
                value={values.timezone_window[edge]}
                onChange={(event) =>
                  set("timezone_window", {
                    ...values.timezone_window,
                    [edge]: Number(event.target.value),
                  })
                }
              >
                {HOURS.map((hour) => (
                  <option key={hour} value={hour}>
                    {t("timezone.hour", { hour })}
                  </option>
                ))}
              </select>
            </label>
          ))}
        </div>
      </fieldset>

      <fieldset className="flex flex-col gap-2">
        <legend className="mb-1 font-medium text-fg">{t("workModes")}</legend>
        <div className="flex flex-wrap gap-2">
          {WORK_MODE_OPTIONS.map((option) => (
            <label
              key={option.value}
              className="flex cursor-pointer items-center gap-2 rounded-xl border border-border bg-surface px-4 py-2 text-sm text-fg has-checked:border-primary has-checked:bg-primary-soft"
            >
              <input
                type="checkbox"
                checked={values.work_modes[option.value]}
                onChange={(event) =>
                  set("work_modes", { ...values.work_modes, [option.value]: event.target.checked })
                }
              />
              {t(option.labelKey)}
            </label>
          ))}
        </div>
      </fieldset>

      <fieldset className="flex flex-col gap-2">
        <legend className="mb-1 font-medium text-fg">{t("languages")}</legend>
        <div className="flex flex-wrap gap-2">
          {WORKING_LANGUAGE_OPTIONS.map((language) => (
            <label
              key={language}
              className="flex cursor-pointer items-center gap-2 rounded-xl border border-border bg-surface px-4 py-2 text-sm text-fg has-checked:border-primary has-checked:bg-primary-soft"
            >
              <input
                type="checkbox"
                checked={values.working_languages.includes(language)}
                onChange={(event) =>
                  set(
                    "working_languages",
                    event.target.checked
                      ? [...values.working_languages, language]
                      : values.working_languages.filter((item) => item !== language),
                  )
                }
              />
              {t(`preferences:language.${language}`)}
            </label>
          ))}
        </div>
      </fieldset>

      <fieldset className="flex flex-col gap-3">
        <legend className="mb-1 font-medium text-fg">{t("dealBreakers")}</legend>
        <ChipField
          id="pref-deal-breakers"
          label={t("dealBreakers.keywords")}
          placeholder={t("dealBreakers.keywords.placeholder")}
          items={values.deal_breaker_keywords}
          onChange={(keywords) => set("deal_breaker_keywords", keywords)}
        />
        <label
          className="flex flex-col gap-1 text-sm text-fg-muted"
          htmlFor="pref-deal-breakers-notes"
        >
          {t("dealBreakers.notes")}
          <textarea
            id="pref-deal-breakers-notes"
            rows={2}
            className={textClass}
            value={values.deal_breaker_notes}
            onChange={(event) => set("deal_breaker_notes", event.target.value)}
          />
        </label>
      </fieldset>

      <ChipField
        id="pref-excluded"
        label={t("excluded")}
        placeholder={t("excluded.placeholder")}
        items={values.excluded_companies}
        onChange={(companies) => set("excluded_companies", companies)}
      />
    </div>
  );
}

/** One free-text list (roles, locations, keywords, companies). */
function ChipField({
  id,
  label,
  placeholder,
  items,
  onChange,
}: {
  id: string;
  label: string;
  placeholder: string;
  items: string[];
  onChange: (next: string[]) => void;
}) {
  const { t } = useTranslation("preferences");
  const [draft, setDraft] = useState("");

  const add = () => {
    const value = draft.trim();
    if (value === "" || items.includes(value)) return;
    onChange([...items, value]);
    setDraft("");
  };

  return (
    <div className="flex flex-col gap-2">
      <label className="font-medium text-fg" htmlFor={id}>
        {label}
      </label>
      <div className="flex gap-2">
        <input
          id={id}
          className={textClass}
          value={draft}
          placeholder={placeholder}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              add();
            }
          }}
        />
        <Button
          type="button"
          variant="outline"
          size="sm"
          aria-label={t("add", { field: label })}
          onClick={add}
        >
          +
        </Button>
      </div>
      {items.length > 0 ? (
        <ul className="flex flex-wrap gap-2">
          {items.map((item) => (
            <li
              key={item}
              className="flex items-center gap-2 rounded-xl bg-surface-muted px-3 py-1 text-sm text-fg"
            >
              {item}
              <button
                type="button"
                aria-label={t("remove", { item })}
                className="text-fg-subtle hover:text-danger"
                onClick={() => onChange(items.filter((entry) => entry !== item))}
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
