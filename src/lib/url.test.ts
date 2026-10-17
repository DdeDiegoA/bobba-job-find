import { describe, expect, it } from "vitest";
import { isHttpsUrl } from "./url";

describe("isHttpsUrl", () => {
  it("accepts an absolute https URL", () => {
    expect(isHttpsUrl("https://jobs.example.com/abc-1")).toBe(true);
    expect(isHttpsUrl("HTTPS://jobs.example.com/abc-1")).toBe(true);
  });

  it("refuses every other scheme", () => {
    expect(isHttpsUrl("http://jobs.example.com/abc-1")).toBe(false);
    expect(isHttpsUrl("javascript:alert(1)")).toBe(false);
    expect(isHttpsUrl("data:text/html,<script>alert(1)</script>")).toBe(false);
    expect(isHttpsUrl("file:///etc/passwd")).toBe(false);
  });

  it("refuses what is not a URL at all", () => {
    expect(isHttpsUrl("")).toBe(false);
    expect(isHttpsUrl("   ")).toBe(false);
    expect(isHttpsUrl(null)).toBe(false);
    expect(isHttpsUrl(undefined)).toBe(false);
    expect(isHttpsUrl("https://")).toBe(false);
    expect(isHttpsUrl("https://exa mple.com")).toBe(false);
    expect(isHttpsUrl("jobs.example.com/abc-1")).toBe(false);
  });
});
