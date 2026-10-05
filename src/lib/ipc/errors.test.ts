import { describe, expect, it } from "vitest";
import type { AppError } from "./bindings";
import { isAppError, renderAppError, UNKNOWN_ERROR_KEY, unwrap } from "./errors";

/** Minimal translator: known key -> text, unknown key -> the key itself. */
const t = (key: string): string => (key === "errors.offer.not_found" ? "Offer not found" : key);

describe("renderAppError", () => {
  it("translates message_key via i18n", () => {
    const error: AppError = {
      code: "not_found",
      message_key: "errors.offer.not_found",
      details: null,
    };
    expect(renderAppError(error, t)).toBe("Offer not found");
  });

  it("never renders the raw message_key or details", () => {
    const error: AppError = {
      code: "internal",
      message_key: "errors.secret.raw",
      details: { secret: "do-not-show" },
    };
    const out = renderAppError(error, t);
    expect(out).not.toContain("errors.secret.raw");
    expect(out).not.toContain("do-not-show");
    expect(out).toBe(UNKNOWN_ERROR_KEY);
  });

  it("falls back to the unknown key when message_key is empty", () => {
    const error: AppError = { code: "internal", message_key: "", details: null };
    expect(renderAppError(error, t)).toBe(UNKNOWN_ERROR_KEY);
  });
});

describe("isAppError", () => {
  it("accepts a valid envelope", () => {
    expect(isAppError({ code: "conflict", message_key: "errors.conflict" })).toBe(true);
  });

  it("rejects values that are not AppErrors", () => {
    expect(isAppError(new Error("boom"))).toBe(false);
    expect(isAppError(null)).toBe(false);
    expect(isAppError({ code: "conflict" })).toBe(false);
  });
});

describe("unwrap", () => {
  it("returns data on ok", () => {
    expect(unwrap({ status: "ok", data: 42 })).toBe(42);
  });

  it("throws the typed AppError on error", () => {
    const error: AppError = { code: "not_found", message_key: "errors.x", details: null };
    expect(() => unwrap({ status: "error", error })).toThrow();
  });
});
