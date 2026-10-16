import { afterEach, describe, expect, it } from "vitest";
import { clearProgress, isOnboarded, loadProgress, markOnboarded, saveProgress } from "./progress";

afterEach(() => {
  localStorage.clear();
});

describe("wizard progress persistence (AC2)", () => {
  it("round-trips the current step and the answers", () => {
    saveProgress({ index: 2, answers: { preferences: { roles: ["sre"] } } });
    expect(loadProgress()).toEqual({ index: 2, answers: { preferences: { roles: ["sre"] } } });
  });

  it("returns null when nothing was saved yet", () => {
    expect(loadProgress()).toBeNull();
  });

  it("ignores a corrupted record instead of crashing the wizard", () => {
    localStorage.setItem("boba.onboarding.progress", "{not json");
    expect(loadProgress()).toBeNull();
    localStorage.setItem("boba.onboarding.progress", JSON.stringify({ index: -1 }));
    expect(loadProgress()).toBeNull();
  });

  it("clears the record once the wizard is done", () => {
    saveProgress({ index: 1, answers: {} });
    clearProgress();
    expect(loadProgress()).toBeNull();
  });

  it("reports first run until onboarding is marked done", () => {
    expect(isOnboarded()).toBe(false);
    markOnboarded();
    expect(isOnboarded()).toBe(true);
  });
});
