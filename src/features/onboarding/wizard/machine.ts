import type { ComponentType } from "react";
import type { Answers, Progress } from "./progress";

/** Props every wizard step receives (EXPERIENCE §4: Back never loses input). */
export interface StepProps {
  /** This step's saved answers, `{}` on first visit. */
  answers: Record<string, unknown>;
  setAnswer: (key: string, value: unknown) => void;
}

/**
 * One wizard step. Later onboarding slices append their own step to
 * `WIZARD_STEPS` and keep their UI inside their own folder.
 */
export interface WizardStep {
  /** Stable id; also the key its answers are stored under. */
  id: string;
  /** i18n key (in the `onboarding` namespace) of the step title. */
  titleKey: string;
  /** Optional steps show "Skip for now" (AC2). */
  optional?: boolean;
  /** `false` keeps Next disabled; `invalidKey` is the inline reason shown. */
  isValid?: (answers: Record<string, unknown>) => boolean;
  invalidKey?: string;
  /**
   * Runs once when Next completes the last step (S3.8: the finish step starts
   * the first Search Run there). Earlier steps never see it.
   */
  onFinish?: () => void;
  Component: ComponentType<StepProps>;
}

/** `resume` shows the "pick up where you left off" banner first (AC2). */
export type Phase = "resume" | "steps";

export interface WizardState {
  phase: Phase;
  /** Index into the step list. */
  index: number;
  answers: Answers;
  done: boolean;
}

const FIRST_STEP: WizardState = { phase: "steps", index: 0, answers: {}, done: false };

export function currentStep(state: WizardState, steps: WizardStep[]): WizardStep {
  return steps[state.index];
}

/** Resume a quit attempt at the saved step, else start at the first step (AC1/AC2). */
export function initialState(steps: WizardStep[], saved: Progress | null): WizardState {
  if (!saved) return FIRST_STEP;
  // Nothing entered on the first step is not an attempt worth resuming.
  if (saved.index === 0 && Object.keys(saved.answers).length === 0) return FIRST_STEP;
  const index = Math.min(saved.index, Math.max(steps.length - 1, 0));
  return { phase: "resume", index, answers: saved.answers, done: false };
}

export function canAdvance(state: WizardState, steps: WizardStep[]): boolean {
  if (state.phase !== "steps" || state.done) return false;
  const step = currentStep(state, steps);
  return step.isValid?.(state.answers[step.id] ?? {}) ?? true;
}

/** Next (and "Skip for now"): forward only when the step is valid (EXPERIENCE §4). */
export function advance(state: WizardState, steps: WizardStep[]): WizardState {
  if (!canAdvance(state, steps)) return state;
  if (state.index >= steps.length - 1) return { ...state, done: true };
  return { ...state, index: state.index + 1 };
}

/** "Skip for now": optional steps only; valid or not, it moves on (AC2). */
export function skip(state: WizardState, steps: WizardStep[]): WizardState {
  if (state.phase !== "steps" || state.done) return state;
  if (!currentStep(state, steps).optional) return state;
  return state.index >= steps.length - 1
    ? { ...state, done: true }
    : { ...state, index: state.index + 1 };
}

/** Back keeps every answer (AC2). No-op on the first step. */
export function back(state: WizardState, steps: WizardStep[]): WizardState {
  if (state.phase !== "steps" || state.index === 0) return state;
  return { ...state, index: Math.min(state.index - 1, steps.length - 1) };
}

export function answer(
  state: WizardState,
  stepId: string,
  key: string,
  value: unknown,
): WizardState {
  return {
    ...state,
    answers: { ...state.answers, [stepId]: { ...state.answers[stepId], [key]: value } },
  };
}

/** Leave the resume banner for the saved step. */
export function resume(state: WizardState): WizardState {
  return { ...state, phase: "steps" };
}

/** Start over: drop the draft and return to the first step. */
export function restart(_state: WizardState): WizardState {
  return FIRST_STEP;
}
