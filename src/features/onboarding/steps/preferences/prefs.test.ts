import { describe, expect, it } from "vitest";
import type { Preferences } from "@/lib/ipc/bindings";
import { preferenceValuesEqual, salaryFloorFromInput } from "./prefs";

const values = (over: Partial<Preferences> = {}): Preferences => ({
  roles: [],
  seniority_min: null,
  seniority_max: null,
  salary_floor: null,
  locations: [],
  timezone_window: { from: 8, to: 18 },
  work_modes: { remote: true, hybrid: false, onsite: false },
  working_languages: ["en"],
  deal_breaker_keywords: [],
  deal_breaker_notes: "",
  excluded_companies: [],
  ...over,
});

describe("Preferences — the core's money shape (AD-17)", () => {
  it("turns the typed amount into integer minor units", () => {
    expect(salaryFloorFromInput("60000", "USD", "year", "hard")).toEqual({
      amount_minor: 6_000_000,
      currency: "USD",
      period: "year",
      firmness: "hard",
    });
    expect(salaryFloorFromInput("2500.5", "EUR", "month", "soft")?.amount_minor).toBe(250_050);
  });

  it("clears the floor instead of storing a bad amount", () => {
    expect(salaryFloorFromInput("", "USD", "year", "hard")).toBeNull();
    expect(salaryFloorFromInput("abc", "USD", "year", "hard")).toBeNull();
    expect(salaryFloorFromInput("-5", "USD", "year", "hard")).toBeNull();
  });
});

describe("Preferences — value comparison", () => {
  it("compares by value, not identity", () => {
    const current = values();
    expect(
      preferenceValuesEqual(current, { ...current, work_modes: { ...current.work_modes } }),
    ).toBe(true);
    expect(preferenceValuesEqual(current, values({ roles: ["SRE"] }))).toBe(false);
  });
});
