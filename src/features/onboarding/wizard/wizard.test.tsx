import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "@/lib/i18n";
import { OnboardingWizard } from "./index";
import type { StepProps, WizardStep } from "./machine";
import { loadProgress, markOnboarded, saveProgress } from "./progress";

/** A stand-in step with one text input, so the framework can be driven for real. */
function PickStep({ answers, setAnswer }: StepProps) {
  return (
    <label>
      Pick
      <input
        value={String(answers.pick ?? "")}
        onChange={(event) => setAnswer("pick", event.target.value)}
      />
    </label>
  );
}

const steps: WizardStep[] = [
  { id: "language", titleKey: "onboarding:steps.language.title", Component: PickStep },
  {
    id: "engine",
    titleKey: "onboarding:wizard.title",
    optional: true,
    isValid: (answers) => answers.pick === "claude",
    invalidKey: "onboarding:wizard.invalid.engine",
    Component: PickStep,
  },
];

const next = () => screen.getByRole("button", { name: "Next" }) as HTMLButtonElement;
const atStep = () => screen.getByRole("progressbar").getAttribute("aria-valuenow");
const pick = () => screen.getByLabelText("Pick") as HTMLInputElement;

afterEach(() => {
  setLocale("en");
  localStorage.clear();
});

describe("wizard framework — first launch (AC1)", () => {
  it("opens as a modal over the app on the first step", () => {
    render(<OnboardingWizard steps={steps} />);
    const dialog = screen.getByRole("dialog", { name: "Boba setup" });
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(within(dialog).getByText("Language & theme")).toBeTruthy();
    expect(within(dialog).getByText("Step 1 of 2")).toBeTruthy();
    expect(atStep()).toBe("1");
  });

  it("stays out of the way once setup is done", () => {
    markOnboarded();
    const { container } = render(<OnboardingWizard steps={steps} />);
    expect(container.textContent).toBe("");
  });
});

describe("wizard framework — navigation (AC2)", () => {
  it("keeps the input when going back and forward again", () => {
    render(<OnboardingWizard steps={steps} />);
    fireEvent.change(pick(), { target: { value: "hello" } });
    fireEvent.click(next());
    expect(atStep()).toBe("2");
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(atStep()).toBe("1");
    expect(pick().value).toBe("hello");
  });

  it("disables Back on the first step", () => {
    render(<OnboardingWizard steps={steps} />);
    expect((screen.getByRole("button", { name: "Back" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("disables Next with an inline reason while the step is invalid (§4)", () => {
    render(<OnboardingWizard steps={steps} />);
    fireEvent.click(next());
    expect(next().disabled).toBe(true);
    const reason = document.getElementById(next().getAttribute("aria-describedby") ?? "");
    expect(reason?.textContent).toBe("wizard.invalid.engine");
  });

  it("offers Skip for now on optional steps only, and skips invalid ones (AC2)", () => {
    render(<OnboardingWizard steps={steps} />);
    expect(screen.queryByRole("button", { name: "Skip for now" })).toBeNull();
    fireEvent.click(next());
    fireEvent.click(screen.getByRole("button", { name: "Skip for now" }));
    // Skipping the last step finishes the wizard.
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByText("Boba setup")).toBeNull();
  });

  it("drops the draft once the wizard is finished", () => {
    saveProgress({ index: 1, answers: {} });
    render(<OnboardingWizard steps={steps} />);
    fireEvent.click(screen.getByRole("button", { name: "Resume setup" }));
    fireEvent.click(screen.getByRole("button", { name: "Skip for now" }));
    expect(loadProgress()).toBeNull();
  });
});

describe("wizard framework — resume (AC2)", () => {
  it("banners a quit attempt and returns to that step with the inputs intact", () => {
    saveProgress({ index: 1, answers: { engine: { pick: "claude" } } });
    render(<OnboardingWizard steps={steps} />);
    expect(screen.getByText("Pick up where you left off")).toBeTruthy();
    expect(screen.getByText(/You stopped at Boba setup/)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Resume setup" }));
    expect(atStep()).toBe("2");
    expect(pick().value).toBe("claude");
  });

  it("starts over on request", () => {
    saveProgress({ index: 1, answers: { engine: { pick: "claude" } } });
    render(<OnboardingWizard steps={steps} />);
    fireEvent.click(screen.getByRole("button", { name: "Start over" }));
    expect(atStep()).toBe("1");
    expect(pick().value).toBe("");
  });

  it("saves progress after every step", () => {
    render(<OnboardingWizard steps={steps} />);
    fireEvent.click(next());
    expect(loadProgress()).toEqual({ index: 1, answers: {} });
  });
});
