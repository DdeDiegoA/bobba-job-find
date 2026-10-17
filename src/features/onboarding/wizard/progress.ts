/** Answers collected by the wizard, keyed by step id (`Answers[stepId][field]`). */
export type Answers = Record<string, Record<string, unknown>>;

/**
 * Wizard progress survives a quit (AC2). Persisted in `localStorage`, like the
 * theme (S1.5) and locale (S1.4) choices; the settings table (S1.7) owns the
 * post-onboarding config, not the in-flight wizard draft.
 */
export interface Progress {
  /** Index into the step list the user was on. */
  index: number;
  answers: Answers;
}

const PROGRESS_KEY = "boba.onboarding.progress";
const DONE_KEY = "boba.onboarding.done";

export function saveProgress(progress: Progress): void {
  try {
    localStorage.setItem(PROGRESS_KEY, JSON.stringify(progress));
  } catch {
    // Storage disabled (e.g. hardening): the run keeps working in memory.
  }
}

/** The saved draft, or `null` when absent or unreadable (never throws). */
export function loadProgress(): Progress | null {
  let raw: string | null;
  try {
    raw = localStorage.getItem(PROGRESS_KEY);
  } catch {
    return null;
  }
  if (!raw) return null;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return null;
    const { index, answers } = parsed as Partial<Progress>;
    if (!Number.isInteger(index) || (index as number) < 0) return null;
    if (typeof answers !== "object" || answers === null) return null;
    return { index: index as number, answers: answers as Answers };
  } catch {
    return null;
  }
}

export function clearProgress(): void {
  try {
    localStorage.removeItem(PROGRESS_KEY);
  } catch {
    // Storage disabled: nothing to clear.
  }
}

/**
 * The local mirror of the core's answer (S3.8): `onboarding_status` is the
 * source of truth, and [`useOnboardingGate`] refreshes this mirror from it on
 * every launch. It exists so Today opens without waiting for a round trip, and
 * so a locked store (which cannot answer) still behaves as it did before.
 */
export function isOnboarded(): boolean {
  try {
    return localStorage.getItem(DONE_KEY) === "1";
  } catch {
    return false;
  }
}

export function markOnboarded(): void {
  try {
    localStorage.setItem(DONE_KEY, "1");
  } catch {
    // Storage disabled: onboarding repeats next launch.
  }
}

/** The core says setup is not done (a reset wiped it): drop the stale mirror. */
export function clearOnboarded(): void {
  try {
    localStorage.removeItem(DONE_KEY);
  } catch {
    // Storage disabled: nothing to clear.
  }
}
