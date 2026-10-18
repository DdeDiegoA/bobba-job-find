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
 * ICU renders the space around a currency symbol as NBSP, narrow NBSP or
 * nothing depending on its version, so exact assertions drop whitespace. The
 * digits and separators themselves are pinned by the currency's exponent.
 */
const noSpace = (value: string): string => value.replace(/\s/g, "");

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
    expect(noSpace(formatMoney(1234, "CLP", "es-CL"))).toBe("$1.234");
    expect(formatMoney(123_400, "USD", "en-US")).toBe("$1,234.00");
  });
});

describe("salary formatting (FR-115)", () => {
  it("shows the amount in its original currency", () => {
    expect(formatMoney(123_400, "USD", "en-US")).toBe("$1,234.00");
    // COP has two minor units per ISO-4217; ICU 77 and ICU 78 disagree on its
    // CLDR digits, so the formatter pins the count instead of trusting them.
    expect(noSpace(formatMoney(123_400, "COP", "es-CO"))).toBe("$1.234,00");
  });

  it("shows a no-decimal currency without centavos and a yen without decimals", () => {
    expect(noSpace(formatMoney(1_234_567, "CLP", "es-CL"))).toBe("$1.234.567");
    expect(noSpace(formatMoney(8_000_000, "JPY", "ja-JP"))).toBe("￥8,000,000");
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
