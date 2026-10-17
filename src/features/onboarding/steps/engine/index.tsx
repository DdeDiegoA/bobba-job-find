import { useMutation, useQuery } from "@tanstack/react-query";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  commands,
  type EngineCard,
  type EngineSnapshot,
  type TestCallResult,
} from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";
import type { StepProps } from "../../wizard/machine";

/**
 * Where the CLI's provider lives, for the data notice (NFR-3). Brand names are
 * not translated; the CLIs that route through the user's own provider say so.
 */
const PROVIDER: Record<string, string> = {
  claude_code: "Anthropic",
  codex: "OpenAI",
  gemini_cli: "Google",
  opencode: "the provider you configured in OpenCode",
  aider: "the provider you configured in Aider",
};

/**
 * Next is allowed only after a passing test and the data notice (FR-3, NFR-3).
 * Limited mode, which lets the user continue without any CLI, is S3.3.
 */
export function engineStepIsValid(answers: Record<string, unknown>): boolean {
  return answers.testedOk === true && answers.disclosureAck === true;
}

/** The result the step last saw, kept in the wizard answers so it survives Back. */
function lastResultOf(answers: Record<string, unknown>): TestCallResult | null {
  return (answers.lastResult as TestCallResult | undefined) ?? null;
}

/**
 * Step — Agent CLI and model (FR-2, FR-3, AC1). Detection, the lockdown verdict
 * and the test call all come from the core; this step only asks, shows and
 * records the answer. The choice is stored by the core on a passing test, so
 * Settings edits the same row later and no other data is touched (FR-17).
 */
export default function EngineStep({ answers, setAnswer }: StepProps) {
  const { t } = useTranslation();
  const [errorText, setErrorText] = useState<string | null>(null);

  const snapshotQuery = useQuery({
    queryKey: ["engine", "snapshot"],
    queryFn: () => commands.agentEngineSnapshot().then(unwrap),
  });
  const snapshot: EngineSnapshot | undefined = snapshotQuery.data;
  const engines = snapshot?.engines ?? [];
  const installed = engines.filter((engine) => engine.installed);
  const missing = engines.filter((engine) => !engine.installed);

  const selectedId =
    (answers.cli as string | undefined) ?? snapshot?.chosen?.cli ?? installed[0]?.cli ?? "";
  const selected = engines.find((engine) => engine.cli === selectedId);
  const model = (answers.model as string | undefined) ?? snapshot?.chosen?.model ?? "";

  const modelsQuery = useQuery({
    queryKey: ["engine", "models", selectedId],
    queryFn: () => commands.agentModels(selectedId).then(unwrap),
    enabled: selectedId !== "",
    staleTime: Number.POSITIVE_INFINITY,
  });
  const listedModels = modelsQuery.data ?? [];

  /** A different CLI or model invalidates the previous pass: test again. */
  const resetTest = () => {
    setAnswer("testedOk", false);
    setAnswer("lastResult", null);
    setErrorText(null);
  };

  const choose = useMutation({
    mutationFn: (result: TestCallResult) =>
      commands
        .agentEngineChoose(result.cli, result.model)
        .then(unwrap)
        .then(() => result),
  });

  const test = useMutation({
    mutationFn: () => {
      const trimmed = model.trim();
      return commands.agentTestCall(selectedId, trimmed === "" ? null : trimmed).then(unwrap);
    },
    onSuccess: (result) => {
      setAnswer("cli", result.cli);
      setAnswer("testedOk", result.ok);
      setAnswer("lastResult", result);
      setErrorText(null);
      // A passing test is the choice Settings edits later (FR-17).
      if (result.ok) choose.mutate(result);
    },
    onError: (error) => {
      setAnswer("testedOk", false);
      setAnswer("lastResult", null);
      setErrorText(
        isAppError(error) ? renderAppError(error, t) : t("onboarding_engine:test.failure"),
      );
    },
  });

  const last = lastResultOf(answers);

  return (
    <div className="flex flex-col gap-6">
      <p className="text-fg-muted">{t("onboarding_engine:step.body")}</p>

      {snapshotQuery.isPending ? (
        <p className="text-sm text-fg-muted">{t("onboarding_engine:step.detecting")}</p>
      ) : null}
      {snapshotQuery.isError ? (
        <p className="text-sm text-danger">
          {isAppError(snapshotQuery.error)
            ? renderAppError(snapshotQuery.error, t)
            : t("onboarding_engine:step.loadError")}
        </p>
      ) : null}
      {errorText ? <p className="text-sm text-danger">{errorText}</p> : null}

      {snapshot && installed.length === 0 ? (
        <p className="text-sm text-fg-muted">{t("onboarding_engine:step.noEngine")}</p>
      ) : null}

      {installed.length > 0 ? (
        <fieldset className="flex flex-col gap-2">
          <legend className="mb-1 font-medium text-fg">
            {t("onboarding_engine:step.installed")}
          </legend>
          {installed.map((engine) => (
            <EngineOption
              key={engine.cli}
              engine={engine}
              checked={selectedId === engine.cli}
              onSelect={() => {
                setAnswer("cli", engine.cli);
                setAnswer("model", "");
                resetTest();
              }}
            />
          ))}
        </fieldset>
      ) : null}

      {missing.length > 0 ? (
        <section className="flex flex-col gap-2">
          <h3 className="font-medium text-fg">{t("onboarding_engine:step.notInstalled")}</h3>
          <ul className="flex flex-col gap-1">
            {missing.map((engine) => (
              <li key={engine.cli} className="flex items-center justify-between gap-2">
                <span className="text-sm text-fg-muted">{engine.name}</span>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    // The webview is not a browser: open the page outside it.
                    void openUrl(engine.install_url).catch(() => undefined);
                  }}
                >
                  {t("onboarding_engine:step.install", { name: engine.name })}
                </Button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {selected ? (
        <>
          {selected.unsupported_tasks.length > 0 ? (
            <p className="text-sm text-warning">
              {t("onboarding_engine:tasks.unavailable", {
                cli: selected.name,
                tasks: selected.unsupported_tasks
                  .map((task) => t(`onboarding_engine:tasks.name.${task}`))
                  .join(", "),
              })}
            </p>
          ) : null}

          <div className="flex flex-col gap-1">
            <label htmlFor="onboarding-engine-model" className="font-medium text-fg">
              {t("onboarding_engine:model.label")}
            </label>
            <input
              id="onboarding-engine-model"
              list="onboarding-engine-models"
              className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
              placeholder={t("onboarding_engine:model.placeholder")}
              value={model}
              onChange={(event) => {
                setAnswer("model", event.target.value);
                resetTest();
              }}
            />
            <datalist id="onboarding-engine-models">
              {listedModels.map((listed) => (
                <option key={listed} value={listed} />
              ))}
            </datalist>
            <p className="text-sm text-fg-muted">
              {listedModels.length > 0
                ? t("onboarding_engine:model.listed")
                : t("onboarding_engine:model.help")}
            </p>
          </div>

          <div className="flex flex-wrap items-center gap-3">
            <Button
              variant="outline"
              disabled={selectedId === "" || test.isPending}
              onClick={() => test.mutate()}
            >
              {test.isPending
                ? t("onboarding_engine:test.running")
                : t("onboarding_engine:test.action")}
            </Button>
          </div>

          {last ? <TestResultView result={last} /> : null}

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
        </>
      ) : null}
    </div>
  );
}

/** One installed CLI: pick it, and see its health at a glance (FR-2). */
function EngineOption({
  engine,
  checked,
  onSelect,
}: {
  engine: EngineCard;
  checked: boolean;
  onSelect: () => void;
}) {
  const { t } = useTranslation();
  return (
    <label className="flex cursor-pointer flex-wrap items-center gap-x-3 gap-y-1 rounded-xl border border-border bg-surface px-4 py-2 text-fg has-checked:border-primary has-checked:bg-primary-soft">
      <input type="radio" name="onboarding-engine" checked={checked} onChange={onSelect} />
      <span className="font-medium">{engine.name}</span>
      <span className="text-sm text-fg-muted">
        {t("onboarding_engine:step.version", { version: engine.version })}
      </span>
      <span className="text-sm text-fg-muted">
        {t(`onboarding_engine:health.${engine.health}`)}
      </span>
      {engine.path ? (
        <span className="w-full font-mono text-xs text-fg-subtle">
          {t("onboarding_engine:step.path", { path: engine.path })}
        </span>
      ) : null}
    </label>
  );
}

/** The test call's outcome: latency and model, or the CLI's error and a hint. */
function TestResultView({ result }: { result: TestCallResult }) {
  const { t } = useTranslation();
  if (result.ok) {
    return (
      <p role="status" className="text-sm text-fg">
        {result.model
          ? t("onboarding_engine:test.success", {
              ms: result.latency_ms,
              model: result.model,
            })
          : t("onboarding_engine:test.successDefault", { ms: result.latency_ms })}
      </p>
    );
  }
  return (
    <div role="status" className="flex flex-col gap-1 text-sm">
      <p className="text-danger">{t("onboarding_engine:test.failure")}</p>
      {result.message_key ? <p className="text-fg">{t(result.message_key)}</p> : null}
      {result.failure_reason ? (
        <p className="text-fg-muted">{t(`onboarding_engine:test.hint.${result.failure_reason}`)}</p>
      ) : null}
      {result.reply ? (
        <p className="whitespace-pre-wrap break-words font-mono text-xs text-fg-subtle">
          {t("onboarding_engine:test.output", { output: result.reply })}
        </p>
      ) : null}
    </div>
  );
}
