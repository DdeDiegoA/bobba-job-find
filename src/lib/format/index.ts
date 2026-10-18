/**
 * Locale-aware formatters (S10.4, FR-115).
 *
 * Salaries always show in their **original** currency, formatted for the UI
 * locale. A converted amount is a second line only: it exists when the user
 * turned FX on, and it always carries the date of the rate it used, so a stale
 * rate can never look like today's. The conversion itself is computed by the
 * core (`boba_app::network_log::fx`); this module re-applies the same rule to
 * the rates it was handed, so the formatters can be tested on their own and no
 * amount is ever converted twice.
 */

/** AD-17 money: integer minor units plus the ISO-4217 code. */
export interface MoneyAmount {
  amount_minor: number;
  currency: string;
}

/** One conversion, already computed by the core, with its rate's date. */
export interface SalaryConversion {
  amount_minor: number;
  currency: string;
  /** The rate's date, `YYYY-MM-DD`, exactly as the source published it. */
  as_of: string;
}

/** The stored FX setting, as `fx_settings` returns it (AC2). */
export interface FxRates {
  enabled: boolean;
  target: string;
  as_of: string | null;
  /** specta maps `f64` values to `number | null`; `null` is never a rate. */
  rates: Record<string, number | null>;
}

/**
 * The ISO-4217 minor-unit exponent of `currency`: how many decimal digits one
 * major unit has. Mirrors `boba_domain::posting::minor_unit_exponent` (AD-17):
 * CLP, JPY and KRW have none and the Gulf currencies have three, so money is
 * never divided by 100 for every currency. An unnamed code falls back to two,
 * the ISO default (and the exponent of USD, EUR and COP).
 */
export function minorUnitExponent(currency: string): number {
  switch (currency.trim().toUpperCase()) {
    case "BIF":
    case "CLP":
    case "DJF":
    case "GNF":
    case "ISK":
    case "JPY":
    case "KMF":
    case "KRW":
    case "PYG":
    case "RWF":
    case "UGX":
    case "UYI":
    case "VND":
    case "VUV":
    case "XAF":
    case "XOF":
    case "XPF":
      return 0;
    case "BHD":
    case "IQD":
    case "JOD":
    case "KWD":
    case "LYD":
    case "OMR":
    case "TND":
      return 3;
    default:
      return 2;
  }
}

/** `10^minorUnitExponent(currency)`. */
export function minorUnitScale(currency: string): number {
  return 10 ** minorUnitExponent(currency);
}

/** Minor units to major units: `123_400` USD cents is `1234`. */
export function majorUnits(amountMinor: number, currency: string): number {
  return amountMinor / minorUnitScale(currency);
}

/**
 * Format an amount in its own currency for `locale`.
 *
 * The digit count is the currency's own exponent, never the runtime's CLDR
 * data: ICU 77 gives COP two decimals and ICU 78 none, which would drop the
 * cents on newer webviews. CLP and JPY still show none; only the exponent comes
 * from the table, so nothing is forced to two.
 */
export function formatMoney(amountMinor: number, currency: string, locale: string): string {
  const exponent = minorUnitExponent(currency);
  return new Intl.NumberFormat(locale, {
    style: "currency",
    currency: currency.toUpperCase(),
    minimumFractionDigits: exponent,
    maximumFractionDigits: exponent,
  }).format(majorUnits(amountMinor, currency));
}

/** Format a salary, in compact form for cards: `$1.2K`. */
export function formatCompactMoney(amountMinor: number, currency: string, locale: string): string {
  return new Intl.NumberFormat(locale, {
    style: "currency",
    currency: currency.toUpperCase(),
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(majorUnits(amountMinor, currency));
}

/** A plain number for `locale`: `1,234,567`. */
export function formatNumber(value: number, locale: string): string {
  return new Intl.NumberFormat(locale).format(value);
}

/** A date label for `locale`; an unparseable value is shown as it came. */
export function formatDateLabel(dateOrIso: string, locale: string): string {
  const dateOnly = /^\d{4}-\d{2}-\d{2}$/.test(dateOrIso);
  const value = new Date(dateOnly ? `${dateOrIso}T00:00:00` : dateOrIso);
  if (Number.isNaN(value.getTime())) return dateOrIso;
  return new Intl.DateTimeFormat(locale, { dateStyle: "medium" }).format(value);
}

/**
 * The salary label: the original amount, plus `≈ <converted> (rate <date>)`
 * when a conversion is supplied (AC2). Without one it is exactly the original.
 */
export function formatSalary(
  amountMinor: number,
  currency: string,
  locale: string,
  conversion?: SalaryConversion | null,
): string {
  const original = formatMoney(amountMinor, currency, locale);
  if (!conversion || conversion.currency.toUpperCase() === currency.toUpperCase()) {
    return original;
  }
  const converted = formatMoney(conversion.amount_minor, conversion.currency, locale);
  return `${original} ≈ ${converted} (${formatDateLabel(conversion.as_of, locale)})`;
}

/**
 * The compact salary label cards use: `$1.2K`, plus `≈ €1.1K (Oct 16, 2026)`
 * when a conversion is supplied (AC2).
 */
export function formatCompactSalary(
  amountMinor: number,
  currency: string,
  locale: string,
  conversion?: SalaryConversion | null,
): string {
  const original = formatCompactMoney(amountMinor, currency, locale);
  if (!conversion || conversion.currency.toUpperCase() === currency.toUpperCase()) {
    return original;
  }
  const converted = formatCompactMoney(conversion.amount_minor, conversion.currency, locale);
  return `${original} ≈ ${converted} (${formatDateLabel(conversion.as_of, locale)})`;
}

/**
 * Convert an amount from the stored rates, under the core's rule: only when FX
 * is enabled, the rate carries a date, the currency is not the display one and
 * the source named it. Everything else returns `null` — nothing is imputed.
 */
export function convertWithRates(
  amountMinor: number,
  currency: string,
  fx: FxRates,
): SalaryConversion | null {
  if (!fx.enabled || !fx.as_of) return null;
  const target = fx.target.trim().toUpperCase();
  if (!target || target === currency.trim().toUpperCase()) return null;
  const rate = fx.rates[currency.trim().toUpperCase()];
  if (typeof rate !== "number" || !Number.isFinite(rate) || rate <= 0) return null;
  // Convert through major units: source and target need not share an exponent.
  const converted = Math.round(
    (amountMinor / minorUnitScale(currency) / rate) * minorUnitScale(target),
  );
  if (!Number.isFinite(converted)) return null;
  return { amount_minor: converted, currency: target, as_of: fx.as_of };
}
