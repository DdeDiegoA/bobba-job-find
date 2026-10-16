import { describe, expect, it } from "vitest";
import {
  advance,
  answer,
  back,
  canAdvance,
  currentStep,
  initialState,
  restart,
  resume,
  skip,
  type WizardStep,
} from "./machine";

const Step = ({ answers }: { answers: Record<string, unknown> }) => <p>{String(answers.pick)}</p>;

const steps: WizardStep[] = [
  {
    id: "language",
    titleKey: "steps.language.title",
    Component: Step,
  },
  {
    id: "engine",
    titleKey: "steps.engine.title",
    optional: true,
    isValid: (answers) => answers.pick === "claude",
    invalidKey: "steps.engine.invalid",
    Component: Step,
  },
];

describe("wizard state machine (AC1/AC2)", () => {
  it("starts on the first step with no answers", () => {
    expect(initialState(steps, null)).toEqual({
      phase: "steps",
      index: 0,
      answers: {},
      done: false,
    });
    expect(currentStep(initialState(steps, null), steps).id).toBe("language");
  });

  it("reopens on the saved step with the saved answers (AC2)", () => {
    const saved = { index: 1, answers: { engine: { pick: "claude" } } };
    const state = initialState(steps, saved);
    expect(state.phase).toBe("resume");
    expect(state.index).toBe(1);
    expect(state.answers).toEqual(saved.answers);
  });

  it("clamps a stale saved index onto the last existing step", () => {
    expect(initialState(steps, { index: 9, answers: {} }).index).toBe(1);
  });

  it("does not banner an untouched draft (AC2)", () => {
    expect(initialState(steps, { index: 0, answers: {} }).phase).toBe("steps");
  });

  it("records inputs per step", () => {
    const state = answer(initialState(steps, null), "language", "locale", "es");
    expect(state.answers).toEqual({ language: { locale: "es" } });
  });

  it("keeps inputs when going back and forward (AC2)", () => {
    let state = initialState(steps, null);
    state = answer(state, "language", "locale", "es");
    state = advance(state, steps);
    state = answer(state, "engine", "pick", "claude");
    state = back(state, steps);
    expect(currentStep(state, steps).id).toBe("language");
    expect(state.answers).toEqual({ language: { locale: "es" }, engine: { pick: "claude" } });
  });

  it("does not go back past the first step", () => {
    const state = initialState(steps, null);
    expect(back(state, steps)).toBe(state);
  });

  it("advances only when the current step is valid (§4)", () => {
    const invalid = initialState(steps, null);
    expect(advance(invalid, steps).index).toBe(1);

    const engine = { ...invalid, index: 1 };
    expect(canAdvance(engine, steps)).toBe(false);
    expect(advance(engine, steps)).toBe(engine);
    expect(canAdvance(answer(engine, "engine", "pick", "claude"), steps)).toBe(true);
  });

  it("treats a step without a validator as valid", () => {
    expect(canAdvance(initialState(steps, null), steps)).toBe(true);
  });

  it("finishes after the last step", () => {
    const last = {
      ...initialState(steps, null),
      index: 1,
      answers: { engine: { pick: "claude" } },
    };
    const done = advance(last, steps);
    expect(done.done).toBe(true);
    expect(canAdvance(done, steps)).toBe(false);
  });

  it("leaves the resume banner for the saved step", () => {
    const state = resume(initialState(steps, { index: 1, answers: {} }));
    expect(state.phase).toBe("steps");
    expect(state.index).toBe(1);
  });

  it("skips an optional step even while it is invalid (AC2)", () => {
    const engine = { ...initialState(steps, null), index: 1 };
    expect(canAdvance(engine, steps)).toBe(false);
    expect(skip(engine, steps).done).toBe(true);
  });

  it("refuses to skip a required step", () => {
    const first = initialState(steps, null);
    expect(skip(first, steps)).toBe(first);
  });

  it("starts over from the first step with a clean draft", () => {
    const state = restart(
      initialState(steps, { index: 1, answers: { engine: { pick: "claude" } } }),
    );
    expect(state).toEqual({ phase: "steps", index: 0, answers: {}, done: false });
  });

  it("ignores step navigation while the resume banner is up", () => {
    const state = initialState(steps, { index: 1, answers: {} });
    expect(advance(state, steps)).toBe(state);
    expect(back(state, steps)).toBe(state);
  });
});
