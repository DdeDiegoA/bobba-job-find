import { describe, expect, it } from "vitest";
import {
  convertWithRates,
  type FxRates,
  formatCompactSalary,
  formatDateLabel,
  formatMoney,
  formatNumber,
  formatSalary,
} from ".";

describe("salary formatting (FR-115)", () => {
  it("shows the amount in its original currency", () => {
    expect(formatMoney(123_400, "USD", "en-US")).toBe("$1,234.00");
    // A different locale formats the same amount its own way.
    expect(formatMoney(123_400, "COP", "es-CO")).toContain("1.234,00");
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
});
