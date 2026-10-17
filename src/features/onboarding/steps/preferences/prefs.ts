import { majorUnits, minorUnitScale } from "@/lib/format";
import type {
  MoneyPeriod,
  Preferences,
  SalaryFirmness,
  SalaryFloor,
  Seniority,
  WorkModes,
} from "@/lib/ipc/bindings";

/**
 * Options and helpers for the Preferences step (S3.7, FR-7). The document
 * itself, its defaults and its versioning live in the core
 * (`commands/preferences.rs`), which stores it encrypted (S1.7) — the UI only
 * shapes what the user sees.
 */
export const SENIORITY_OPTIONS: readonly { value: Seniority; labelKey: string }[] = [
  { value: "intern", labelKey: "preferences:seniority.intern" },
  { value: "junior", labelKey: "preferences:seniority.junior" },
  { value: "mid", labelKey: "preferences:seniority.mid" },
  { value: "senior", labelKey: "preferences:seniority.senior" },
  { value: "lead", labelKey: "preferences:seniority.lead" },
  { value: "principal", labelKey: "preferences:seniority.principal" },
];

export const WORK_MODE_OPTIONS: readonly { value: keyof WorkModes; labelKey: string }[] = [
  { value: "remote", labelKey: "preferences:workMode.remote" },
  { value: "hybrid", labelKey: "preferences:workMode.hybrid" },
  { value: "onsite", labelKey: "preferences:workMode.onsite" },
];

export const MONEY_PERIODS: readonly MoneyPeriod[] = ["year", "month", "hour"];

/** Hard excludes Offers below the floor, soft only lowers the score (FR-7). */
export const SALARY_FIRMNESS_OPTIONS: readonly { value: SalaryFirmness; labelKey: string }[] = [
  { value: "hard", labelKey: "preferences:salary.firmness.hard" },
  { value: "soft", labelKey: "preferences:salary.firmness.soft" },
];

/** ISO-4217 codes offered as the salary currency (AD-17); USD is the default. */
export const CURRENCY_OPTIONS: readonly string[] = [
  "USD",
  "EUR",
  "GBP",
  "COP",
  "MXN",
  "BRL",
  "ARS",
  "CLP",
];

/** The shipped content languages (AD-17); Boba's documents are EN/ES. */
export const WORKING_LANGUAGE_OPTIONS: readonly string[] = ["en", "es"];

/** The hours of the timezone window, as the selects offer them. */
export const HOURS: readonly number[] = Array.from({ length: 24 }, (_, hour) => hour);

/** Value equality, used to tell a real edit from a re-render. */
export function preferenceValuesEqual(a: Preferences, b: Preferences): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * The salary floor as the user typed it, in major units, to the stored shape:
 * integer minor units (AD-17), scaled by the currency's own ISO-4217 exponent
 * (CLP and JPY have none, so their amount is stored as typed). An empty,
 * negative or unparsable amount clears the floor rather than storing a wrong
 * one; the core re-checks it anyway.
 */
export function salaryFloorFromInput(
  amount: string,
  currency: string,
  period: MoneyPeriod,
  firmness: SalaryFirmness,
): SalaryFloor | null {
  const major = Number(amount.trim());
  if (amount.trim() === "" || !Number.isFinite(major) || major < 0) return null;
  return {
    amount_minor: Math.round(major * minorUnitScale(currency)),
    currency,
    period,
    firmness,
  };
}

/** The stored salary floor back in major units, as the amount field edits it. */
export function salaryFloorToInput(floor: SalaryFloor | null): string {
  return floor ? String(majorUnits(floor.amount_minor, floor.currency)) : "";
}
