import { useMutation } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import EnginePicker, { PROVIDER } from "@/features/engine-picker";
import { commands, type TestCallResult } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";
import type { StepProps } from "../../wizard/machine";

/**
 * Next is allowed after a passing test and the data notice (FR-3, NFR-3), or
 * after the user chooses Limited mode (S3.3): a CLI that cannot pass the test
 * must never dead-end the wizard (FIX1). Limited mode is an honest exit, not a
 * pass, so `testedOk` stays false.
 */
export function engineStepIsValid(answers: Record<string, unknown>): boolean {
  if (answers.limitedMode === true) return true;
  return answers.testedOk === true && answers.disclosureAck === true;
}

/** The result the step last saw, kept in the wizard answers so it survives Back. */
function lastResultOf(answers: Record<string, unknown>): TestCallResult | null {
  return (answers.lastResult as TestCallResult | undefined) ?? null;
}

/**
 * Step — Agent CLI and model (FR-2, FR-3, AC1). The picker is shared with
 * Settings → AI engine (FX7); this step adds what only onboarding needs: the
 * data notice (NFR-3) and the Limited-mode way on (S3.3). The choice is stored
 * by the core on a passing test, so Settings edits the same row later and no
 * other data is touched (FR-17).
 */
export default function EngineStep({ answers, setAnswer }: StepProps) {
  const { t } = useTranslation();

  const choose = useMutation({
    mutationFn: (result: TestCallResult) =>
      commands
        .agentEngineChoose(result.cli, result.model)
        .then(unwrap)
        .then(() => result),
  });

  /** A different CLI or model invalidates the previous pass: test again. */
  const resetTest = () => {
    setAnswer("testedOk", false);
    setAnswer("limitedMode", false);
    setAnswer("lastResult", null);
  };

  return (
    <div className="flex flex-col gap-6">
      <p className="text-fg-muted">{t("onboarding_engine:step.body")}</p>

      <EnginePicker
        initialCli={answers.cli as string | undefined}
        initialModel={answers.model as string | undefined}
        initialResult={lastResultOf(answers)}
        onChange={(pick) => {
          setAnswer("cli", pick.cli);
          setAnswer("model", pick.model);
          resetTest();
        }}
        onTested={(result) => {
          setAnswer("cli", result.cli);
          setAnswer("model", result.model ?? "");
          setAnswer("testedOk", result.ok);
          setAnswer("limitedMode", false);
          setAnswer("lastResult", result);
          // A passing test is the choice Settings edits later (FR-17).
          if (result.ok) choose.mutate(result);
        }}
        onTestError={() => {
          setAnswer("testedOk", false);
          setAnswer("lastResult", null);
        }}
      >
        {(selected) =>
          selected ? (
            <section className="flex flex-col gap-2 rounded-xl border border-border bg-surface-muted p-4">
              <h3 className="font-medium text-fg">{t("onboarding_engine:disclosure.title")}</h3>
              <p className="text-sm text-fg-muted">
                {t("onboarding_engine:disclosure.body", {
                  cli: selected.name,
                  provider: PROVIDER[selected.cli] ?? "",
                })}
              </p>
              <label className="flex items-center gap-2 text-sm text-fg">
                <input
                  type="checkbox"
                  checked={answers.disclosureAck === true}
                  onChange={(event) => setAnswer("disclosureAck", event.target.checked)}
                />
                {t("onboarding_engine:disclosure.ack")}
              </label>
            </section>
          ) : null
        }
      </EnginePicker>

      {/* S3.3 (FIX1): a CLI that cannot pass the test — or no CLI at all — must
          not dead-end the wizard. Limited mode is an honest exit: no pass is
          recorded, so every AI feature stays off until an engine is connected. */}
      {answers.limitedMode === true ? (
        <p role="status" className="text-sm text-fg-muted">
          {t("onboarding_engine:limited.chosen")}
        </p>
      ) : (
        <section className="flex flex-col gap-1">
          <Button
            variant="ghost"
            onClick={() => {
              setAnswer("limitedMode", true);
              setAnswer("testedOk", false);
              setAnswer("lastResult", null);
            }}
          >
            {t("onboarding_engine:limited.action")}
          </Button>
          <p className="text-sm text-fg-muted">{t("onboarding_engine:limited.body")}</p>
        </section>
      )}
    </div>
  );
}
