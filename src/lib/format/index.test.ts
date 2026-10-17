import { describe, expect, it } from "vitest";
import {
  convertWithRates,
  type FxRates,
  formatCompactSalary,
  formatDateLabel,
  formatMoney,
  formatNumber,
  formatSalary,
  majorUnits,
  minorUnitExponent,
} from ".";

/**
 * ICU renders the spaces around a currency symbol either as NBSP or as narrow
 * NBSP depending on its version, so assertions compare with them normalised.
 * The digits and separators themselves do not change.
 */
const spaced = (value: string): string => value.replace(/[\s  ]/g, " ");

describe("money's ISO-4217 minor units (AD-17)", () => {
  it("uses the currency's own exponent, never 100 for everything", () => {
    expect(minorUnitExponent("USD")).toBe(2);
    expect(minorUnitExponent("COP")).toBe(2);
    expect(minorUnitExponent("CLP")).toBe(0);
    expect(minorUnitExponent("JPY")).toBe(0);
    expect(minorUnitExponent("KRW")).toBe(0);
    expect(minorUnitExponent("KWD")).toBe(3);
    expect(majorUnits(123_400, "USD")).toBe(1234);
    expect(majorUnits(1234, "CLP")).toBe(1234);
    expect(majorUnits(1_234_500, "KWD")).toBe(1234.5);
  });

  it("formats CLP with no centavos and USD with cents", () => {
    // Symbol spacing is ICU's (es-CL uses neither a space nor a decimal comma).
    expect(spaced(formatMoney(1234, "CLP", "es-CL"))).toContain("1.234");
    expect(spaced(formatMoney(1234, "CLP", "es-CL"))).not.toContain(",");
    expect(formatMoney(123_400, "USD", "en-US")).toBe("$1,234.00");
  });
});

describe("salary formatting (FR-115)", () => {
  it("shows the amount in its original currency", () => {
    expect(formatMoney(123_400, "USD", "en-US")).toBe("$1,234.00");
    // A different locale formats the same amount its own way. Grouping is
    // asserted instead of the exact string: ICU 77 gives COP two decimals and
    // ICU 78 none, so the digit count belongs to the runtime's CLDR data.
    expect(spaced(formatMoney(123_400, "COP", "es-CO"))).toContain("1.234");
    expect(formatMoney(123_400, "COP", "es-CO")).not.toContain("1234");
  });

  it("shows a no-decimal currency without centavos and a yen without decimals", () => {
    expect(spaced(formatMoney(1_234_567, "CLP", "es-CL"))).toContain("1.234.567");
    expect(spaced(formatMoney(8_000_000, "JPY", "ja-JP"))).toContain("8,000,000");
    expect(spaced(formatMoney(8_000_000, "JPY", "ja-JP"))).not.toContain(".");
  });

  it("never converts without a rate", () => {
    expect(formatSalary(100_000, "USD", "en-US")).toBe("$1,000.00");
    expect(formatSalary(100_000, "USD", "en-US", null)).not.toContain("≈");
  });

  it("labels a conversion with the date of the rate", () => {
    const label = formatSalary(100_000, "USD", "en-US", {
      amount_minor: 90_909,
      currency: "EUR",
      as_of: "2026-10-16",
    });

    expect(label.startsWith("$1,000.00")).toBe(true);
    expect(label).toContain("≈");
    expect(label).toContain("€909.09");
    expect(label).toContain("Oct 16, 2026");
  });

  it("keeps the card label compact and still dates the conversion", () => {
    const label = formatCompactSalary(1_234_000, "USD", "en-US", {
      amount_minor: 1_121_818,
      currency: "EUR",
      as_of: "2026-10-16",
    });

    expect(label.startsWith("$12.3K")).toBe(true);
    expect(label).toContain("≈");
    expect(label).toContain("€11.2K");
    expect(label).toContain("Oct 16, 2026");
  });

  it("formats dates and numbers for the UI locale", () => {
    expect(formatDateLabel("2026-10-16", "en-US")).toBe("Oct 16, 2026");
    expect(formatDateLabel("not a date", "en-US")).toBe("not a date");
    expect(formatNumber(1_234_567, "en-US")).toBe("1,234,567");
  });
});

describe("conversion from the stored rates (AC2)", () => {
  const fx: FxRates = {
    enabled: true,
    target: "EUR",
    as_of: "2026-10-16",
    rates: { USD: 1.1 },
  };

  it("converts only when FX is enabled and the currency is covered", () => {
    expect(convertWithRates(100_000, "USD", fx)).toEqual({
      amount_minor: 90_909,
      currency: "EUR",
      as_of: "2026-10-16",
    });
    expect(convertWithRates(100_000, "USD", { ...fx, enabled: false })).toBeNull();
    expect(convertWithRates(100_000, "JPY", fx)).toBeNull();
    expect(convertWithRates(100_000, "EUR", fx)).toBeNull();
    expect(convertWithRates(100_000, "USD", { ...fx, as_of: null })).toBeNull();
  });

  it("converts across currencies with different minor units", () => {
    // 1,000.00 USD at 1 CLP = 0.0009 USD is 1,111,111 CLP, which has no centavos.
    const toClp: FxRates = {
      enabled: true,
      target: "CLP",
      as_of: "2026-10-16",
      rates: { USD: 0.0009 },
    };
    expect(convertWithRates(100_000, "USD", toClp)).toEqual({
      amount_minor: 1_111_111,
      currency: "CLP",
      as_of: "2026-10-16",
    });
  });
});
