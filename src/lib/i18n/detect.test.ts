import { describe, expect, it } from "vitest";
import { pickLocale, resolveLocale } from "./detect";

describe("resolveLocale (AC1)", () => {
  it("maps any es* tag to Spanish", () => {
    for (const tag of ["es", "es-MX", "ES-es", "es-419"]) {
      expect(resolveLocale(tag)).toBe("es");
    }
  });

  it("falls back to English for everything else", () => {
    for (const tag of ["en", "en-US", "fr", "pt-BR", "", null, undefined]) {
      expect(resolveLocale(tag)).toBe("en");
    }
  });
});

describe("pickLocale (AC2)", () => {
  it("prefers a persisted choice over the OS locale", () => {
    expect(pickLocale("es", "en-US")).toBe("es");
    expect(pickLocale("en", "es-MX")).toBe("en");
  });

  it("falls back to the OS locale when nothing valid is stored", () => {
    expect(pickLocale(null, "es-MX")).toBe("es");
    expect(pickLocale("xx", "en-GB")).toBe("en");
    expect(pickLocale(undefined, undefined)).toBe("en");
  });
});
