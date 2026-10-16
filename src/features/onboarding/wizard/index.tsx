import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
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

/**
 * Resumable first-run wizard: stepper + one step panel + footer, in a modal
 * over the app (AC1). Progress is written after every step and a quit attempt
 * reopens on a resume banner (AC2). `steps` is injectable so the framework is
 * tested on its own; the app always renders the real order.
 *
 * shortcut: no focus trap — `/onboarding` renders nothing behind the overlay;
 * add one when the wizard mounts over the shell.
 */
export function OnboardingWizard({ steps = WIZARD_STEPS }: { steps?: WizardStep[] } = {}) {
  const { t } = useTranslation();
  const [state, setState] = useState<WizardState>(() => initialState(steps, loadProgress()));

  useEffect(() => {
    if (state.done) {
      markOnboarded();
      clearProgress();
      return;
    }
    // A resume banner changes nothing yet: only the steps phase is a draft.
    if (state.phase === "steps") saveProgress({ index: state.index, answers: state.answers });
  }, [state]);

  if (isOnboarded() || state.done) return null;

  const step = currentStep(state, steps);
  const blocked = !canAdvance(state, steps);
  const reasonId = "onboarding-step-reason";

  return (
    <div className="fixed inset-0 z-50 grid place-items-center overflow-y-auto bg-bg/95 p-6">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="onboarding-wizard-title"
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

        <ol
          role="progressbar"
          aria-label={t("onboarding:wizard.progressAria")}
          aria-valuemin={1}
          aria-valuenow={state.index + 1}
          aria-valuemax={steps.length}
          className="flex gap-2"
        >
          {steps.map((item, index) => (
            <li
              key={item.id}
              aria-label={t(`onboarding:wizard.step.${stepStatus(index, state.index)}`)}
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
                  onClick={() => setState((prev) => advance(prev, steps))}
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

function stepStatus(index: number, current: number): "done" | "current" | "upcoming" {
  if (index < current) return "done";
  return index === current ? "current" : "upcoming";
}
