import { useMutation, useQuery } from "@tanstack/react-query";
import { openUrl } from "@tauri-apps/plugin-opener";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  commands,
  type EngineCard,
  type EngineChoice,
  type TestCallResult,
} from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";

/**
 * Where the CLI's provider lives, for the data notice (NFR-3). Brand names are
 * not translated; the CLIs that route through the user's own provider say so.
 */
export const PROVIDER: Record<string, string> = {
  claude_code: "Anthropic",
  codex: "OpenAI",
  gemini_cli: "Google",
  opencode: "the provider you configured in OpenCode",
  aider: "the provider you configured in Aider",
};

/** The CLI and model the picker currently has selected. */
export interface EnginePick {
  cli: string;
  model: string;
}

export interface EnginePickerProps {
  /** Seed from a saved draft; absent means the stored choice, then the first installed CLI. */
  initialCli?: string;
  initialModel?: string;
  /** A result to show before any test in this mount (a draft's last pass or failure). */
  initialResult?: TestCallResult | null;
  /** Any CLI or model change invalidates the previous pass. */
  onChange?: (pick: EnginePick) => void;
  /** A test call finished; `result.ok` tells whether it passed. */
  onTested?: (result: TestCallResult) => void;
  /** A test call could not run at all (probe or IPC failure). */
  onTestError?: () => void;
  /** Show Save, enabled only after a passing test, and hand back what passed. */
  onSave?: (choice: EngineChoice) => void;
  saving?: boolean;
  /** Extra content under the result, given the selected engine (the disclosure notice). */
  children?: (selected: EngineCard | undefined) => ReactNode;
}

/**
 * Pick the agent CLI and model, test them, and (when the host asks for it) save
 * the pair that passed (FR-2, FR-3, FR-17). Shared by onboarding's engine step
 * and Settings → AI engine: detection, the lockdown verdict and the test call
 * all come from the core; this component only asks, shows and reports.
 *
 * Choosing is a two-step decision. A test that passes proves the CLI can run;
 * only the host decides when that becomes the stored choice — onboarding stores
 * it on the pass, Settings waits for Save (FX7).
 */
export default function EnginePicker({
  initialCli,
  initialModel,
  initialResult = null,
  onChange,
  onTested,
  onTestError,
  onSave,
  saving = false,
  children,
}: EnginePickerProps) {
  const { t } = useTranslation();
  const [errorText, setErrorText] = useState<string | null>(null);
  const [selection, setSelection] = useState<EnginePick | null>(
    initialCli ? { cli: initialCli, model: initialModel ?? "" } : null,
  );
  const [result, setResult] = useState<TestCallResult | null>(initialResult);

  const snapshotQuery = useQuery({
    queryKey: ["engine", "snapshot"],
    queryFn: () => commands.agentEngineSnapshot().then(unwrap),
  });
  const snapshot = snapshotQuery.data;
  const engines = snapshot?.engines ?? [];
  const installed = engines.filter((engine) => engine.installed);
  const missing = engines.filter((engine) => !engine.installed);

  const chosen = snapshot?.chosen ?? null;
  const selectedId = selection?.cli ?? chosen?.cli ?? installed[0]?.cli ?? "";
  const model =
    selection?.model ?? (chosen && chosen.cli === selectedId ? (chosen.model ?? "") : "");
  const selected = engines.find((engine) => engine.cli === selectedId);

  const modelsQuery = useQuery({
    queryKey: ["engine", "models", selectedId],
    queryFn: () => commands.agentModels(selectedId).then(unwrap),
    enabled: selectedId !== "",
    staleTime: Number.POSITIVE_INFINITY,
  });
  const listedModels = modelsQuery.data ?? [];

  /** A different CLI or model invalidates the previous pass: test again. */
  const change = (next: EnginePick) => {
    setSelection(next);
    setResult(null);
    setErrorText(null);
    onChange?.(next);
  };

  const test = useMutation({
    mutationFn: () => {
      const trimmed = model.trim();
      return commands.agentTestCall(selectedId, trimmed === "" ? null : trimmed).then(unwrap);
    },
    onSuccess: (tested) => {
      setResult(tested);
      setErrorText(null);
      onTested?.(tested);
    },
    onError: (error) => {
      setResult(null);
      setErrorText(
        isAppError(error) ? renderAppError(error, t) : t("onboarding_engine:test.failure"),
      );
      onTestError?.();
    },
  });

  return (
    <>
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
              onSelect={() => change({ cli: engine.cli, model: "" })}
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
            <label htmlFor="engine-picker-model" className="font-medium text-fg">
              {t("onboarding_engine:model.label")}
            </label>
            <input
              id="engine-picker-model"
              list="engine-picker-models"
              className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
              placeholder={t("onboarding_engine:model.placeholder")}
              value={model}
              onChange={(event) => change({ cli: selectedId, model: event.target.value })}
            />
            <datalist id="engine-picker-models">
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
            {onSave ? (
              <Button
                type="button"
                disabled={result?.ok !== true || saving || test.isPending}
                onClick={() => {
                  if (result?.ok === true) onSave({ cli: result.cli, model: result.model });
                }}
              >
                {saving ? t("onboarding_engine:save.saving") : t("onboarding_engine:save.action")}
              </Button>
            ) : null}
          </div>

          {result ? <TestResultView result={result} cliName={selected.name} /> : null}

          {children ? children(selected) : null}
        </>
      ) : null}
    </>
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
      <input type="radio" name="engine-picker" checked={checked} onChange={onSelect} />
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
function TestResultView({ result, cliName }: { result: TestCallResult; cliName?: string }) {
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
      <p className="text-danger">
        {cliName
          ? t("onboarding_engine:test.failureOn", { cli: cliName })
          : t("onboarding_engine:test.failure")}
      </p>
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
