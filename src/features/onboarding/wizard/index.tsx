import { type KeyboardEvent, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { commands } from "@/lib/ipc/bindings";
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
  type WizardState,
  type WizardStep,
} from "./machine";
import { clearProgress, isOnboarded, loadProgress, markOnboarded, saveProgress } from "./progress";
import { WIZARD_STEPS } from "./steps";

/** Focusable controls inside the dialog, in tab order. */
const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * The overlay is the only interactive surface while it is up, so Tab wraps
 * inside it instead of reaching the app behind (DESIGN.md §10.3, EXPERIENCE §7).
 */
function trapTab(event: KeyboardEvent<HTMLDivElement>) {
  if (event.key !== "Tab") return;
  const controls = [...event.currentTarget.querySelectorAll<HTMLElement>(FOCUSABLE)];
  const first = controls[0];
  const last = controls.at(-1);
  if (!first || !last) return;

  const active = document.activeElement;
  if (event.shiftKey && (active === first || active === event.currentTarget)) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && active === last) {
    event.preventDefault();
    first.focus();
  }
}

/**
 * Resumable first-run wizard: stepper + one step panel + footer, in a modal
 * over the app (AC1). Progress is written after every step and a quit attempt
 * reopens on a resume banner (AC2). `steps` is injectable so the framework is
 * tested on its own; the app always renders the real order.
 */
export function OnboardingWizard({
  steps = WIZARD_STEPS,
  onFinished,
}: {
  steps?: WizardStep[];
  /** Called once setup is complete, so the route can hand over to Today. */
  onFinished?: () => void;
} = {}) {
  const { t } = useTranslation();
  const [state, setState] = useState<WizardState>(() => initialState(steps, loadProgress()));
  const dialogRef = useRef<HTMLDivElement>(null);
  const handedOver = useRef(false);

  // The wizard answers the keyboard from the moment it opens (§10.3).
  useEffect(() => {
    dialogRef.current?.focus();
  }, []);

  useEffect(() => {
    if (!state.done) {
      // A resume banner changes nothing yet: only the steps phase is a draft.
      if (state.phase === "steps") saveProgress({ index: state.index, answers: state.answers });
      return;
    }
    markOnboarded();
    clearProgress();
    if (handedOver.current) return;
    handedOver.current = true;
    // Hand over exactly once, after the core has recorded the completion: the
    // first-launch gate reads `onboarding_status`, and the local mirror above
    // only lets Today open without waiting for that round trip.
    void finishInCore(onFinished);
  }, [state, onFinished]);

  /**
   * Next, with the last step's own finish action: completing setup is the one
   * place a step may do something beyond recording an answer (S3.8).
   */
  const goNext = () => {
    const step = currentStep(state, steps);
    const next = advance(state, steps);
    if (next === state) return;
    setState(next);
    if (next.done) step.onFinish?.();
  };

  if (isOnboarded() || state.done) return null;

  const step = currentStep(state, steps);
  const blocked = !canAdvance(state, steps);
  const reasonId = "onboarding-step-reason";

  return (
    <div className="fixed inset-0 z-50 grid place-items-center overflow-y-auto bg-bg/95 p-6">
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="onboarding-wizard-title"
        tabIndex={-1}
        onKeyDown={trapTab}
        className="flex w-full max-w-2xl flex-col gap-6 rounded-card border border-border bg-surface p-6 shadow-[var(--elev-2)]"
      >
        <header className="flex flex-wrap items-baseline justify-between gap-2">
          <h1 id="onboarding-wizard-title" className="font-display text-xl text-fg">
            {t("onboarding:wizard.title")}
          </h1>
          <p className="text-sm text-fg-muted">
            {t("onboarding:wizard.stepOf", { current: state.index + 1, total: steps.length })}
          </p>
        </header>

        <ol aria-label={t("onboarding:wizard.progressAria")} className="flex gap-2">
          {steps.map((item, index) => (
            <li
              key={item.id}
              aria-label={t(`onboarding:wizard.step.${stepStatus(index, state.index)}`)}
              aria-current={index === state.index ? "step" : undefined}
              className={`h-2 flex-1 rounded-full ${
                index <= state.index ? "bg-primary" : "bg-surface-muted"
              }`}
            />
          ))}
        </ol>

        {state.phase === "resume" ? (
          <section className="flex flex-col gap-3">
            <h2 className="font-display text-lg text-fg">{t("onboarding:wizard.resume.title")}</h2>
            <p className="text-fg-muted">
              {t("onboarding:wizard.resume.body", { step: t(step.titleKey) })}
            </p>
            <div className="flex flex-wrap gap-2">
              <Button onClick={() => setState((prev) => resume(prev))}>
                {t("onboarding:wizard.resume.action")}
              </Button>
              <Button variant="ghost" onClick={() => setState((prev) => restart(prev))}>
                {t("onboarding:wizard.resume.restart")}
              </Button>
            </div>
          </section>
        ) : (
          <>
            <h2 className="font-display text-2xl text-fg">{t(step.titleKey)}</h2>
            <step.Component
              answers={state.answers[step.id] ?? {}}
              setAnswer={(key, value) => setState((prev) => answer(prev, step.id, key, value))}
            />
            <footer className="flex flex-wrap items-center justify-between gap-2 border-t border-border pt-4">
              {step.optional ? (
                <Button variant="ghost" onClick={() => setState((prev) => skip(prev, steps))}>
                  {t("onboarding:wizard.skip")}
                </Button>
              ) : (
                <span />
              )}
              <div className="flex items-center gap-2">
                {blocked && step.invalidKey ? (
                  <p id={reasonId} role="status" className="text-sm text-danger">
                    {t(step.invalidKey)}
                  </p>
                ) : null}
                <Button
                  variant="ghost"
                  onClick={() => setState((prev) => back(prev, steps))}
                  disabled={state.index === 0}
                >
                  {t("onboarding:wizard.back")}
                </Button>
                <Button
                  onClick={goNext}
                  disabled={blocked}
                  aria-describedby={blocked && step.invalidKey ? reasonId : undefined}
                >
                  {t("onboarding:wizard.next")}
                </Button>
              </div>
            </footer>
          </>
        )}
      </div>
    </div>
  );
}

/**
 * Record the finish in the core, then hand over (S3.8). A core that cannot
 * answer — a locked store, or a unit test without the Tauri bridge — still
 * hands over: the gate falls back to the local mirror.
 */
async function finishInCore(onFinished?: () => void): Promise<void> {
  try {
    await commands.onboardingComplete();
  } catch {
    // The mirror is the fallback; the next launch asks the core again.
  }
  onFinished?.();
}

function stepStatus(index: number, current: number): "done" | "current" | "upcoming" {
  if (index < current) return "done";
  return index === current ? "current" : "upcoming";
}
