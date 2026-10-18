import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import EnginePicker from "@/features/engine-picker";
import { commands, type EngineChoice, type TestCallResult } from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";

/**
 * Settings → AI engine (S2.5, FR-20) with usage display (PRD §6.3).
 *
 * Four things live here, all decided by the core:
 *
 * * **the engine** — the shared picker (FX7): between them, onboarding's engine
 *   step and this page are the only two ways to pick the CLI and model. Save
 *   is enabled only after a passing test, so a pair that cannot answer is never
 *   stored. Saving refreshes the capability snapshot, which is what leaves
 *   Limited mode (FR-4).
 * * **the per-run offer budget** — how many Offers one Search Run may hand to
 *   the agent (default 30). The value is one `setting` row the daily run reads;
 *   the page only edits it, clamping to the range the core enforces.
 * * **the per-task limits** — turns, MCP tool calls and wall-clock time every
 *   task kind starts with (architecture §5.4). Read-only: they are the core's
 *   defaults, not a second source of truth.
 * * **usage** — the tokens and cost the CLI reported for the round-trip test
 *   call. A CLI that reports nothing says so ("Not reported"), never a zero.
 */
export default function SettingsAiEngine() {
  const { t, t: tError } = useTranslation("ai_engine");
  const queryClient = useQueryClient();
  const budget = useQuery({
    queryKey: ["agent", "budget"],
    queryFn: () => commands.agentBudgetSnapshot().then(unwrap),
  });
  const engine = useQuery({
    queryKey: ["engine", "snapshot"],
    queryFn: () => commands.agentEngineSnapshot().then(unwrap),
  });

  const [draft, setDraft] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const stored = budget.data;
  const value = draft ?? (stored ? String(stored.offers_per_run) : "");
  const parsed = Number.parseInt(value, 10);
  const inRange =
    stored !== undefined &&
    Number.isInteger(parsed) &&
    parsed >= 1 &&
    parsed <= stored.max_offers_per_run;
  const dirty = stored !== undefined && inRange && parsed !== stored.offers_per_run;

  const save = useMutation({
    mutationFn: (offersPerRun: number) => commands.agentBudgetSave(offersPerRun).then(unwrap),
    onSuccess: () => {
      setSaved(true);
      setDraft(null);
      void budget.refetch();
    },
    onError: (error) =>
      setSaveError(isAppError(error) ? renderAppError(error, tError) : t("budget.error.save")),
  });

  const [engineSaved, setEngineSaved] = useState(false);
  const [engineError, setEngineError] = useState<string | null>(null);
  const choose = useMutation({
    mutationFn: (choice: EngineChoice) =>
      commands.agentEngineChoose(choice.cli, choice.model).then(unwrap),
    onSuccess: async () => {
      setEngineSaved(true);
      setEngineError(null);
      // The stored choice feeds Usage here and the capability gate everywhere
      // else: refetching both is what makes a Saved engine leave Limited mode.
      await queryClient.invalidateQueries({ queryKey: ["engine", "snapshot"] });
      await queryClient.invalidateQueries({ queryKey: ["capabilities", "snapshot"] });
    },
    onError: (error) => {
      setEngineSaved(false);
      setEngineError(isAppError(error) ? renderAppError(error, tError) : t("engine.error"));
    },
  });

  return (
    <section className="flex flex-col gap-6 p-6" aria-labelledby="settings-ai-engine-title">
      <div className="flex flex-col gap-1">
        <h1 id="settings-ai-engine-title" className="font-display text-2xl text-fg">
          {t("title")}
        </h1>
        <p className="text-sm text-fg-muted">{t("intro")}</p>
      </div>

      <section className="flex flex-col gap-3" aria-labelledby="ai-engine-engine-title">
        <h2 id="ai-engine-engine-title" className="font-display text-lg text-fg">
          {t("engine.title")}
        </h2>
        <p className="text-sm text-fg-muted">{t("engine.body")}</p>
        {engineError ? <p className="text-sm text-danger">{engineError}</p> : null}
        {engineSaved ? (
          <p role="status" className="text-sm text-fg-muted">
            {t("engine.saved")}
          </p>
        ) : null}
        <EnginePicker
          onChange={() => {
            setEngineSaved(false);
            setEngineError(null);
          }}
          onSave={(choice) => {
            setEngineSaved(false);
            setEngineError(null);
            choose.mutate(choice);
          }}
          saving={choose.isPending}
        />
      </section>

      <section className="flex flex-col gap-2" aria-labelledby="ai-engine-budget-title">
        <h2 id="ai-engine-budget-title" className="font-display text-lg text-fg">
          {t("budget.title")}
        </h2>
        <p className="text-sm text-fg-muted">{t("budget.body")}</p>

        {budget.isPending ? <p className="text-sm text-fg-muted">{t("budget.loading")}</p> : null}
        {budget.isError ? (
          <p className="text-sm text-danger">
            {isAppError(budget.error)
              ? renderAppError(budget.error, tError)
              : t("budget.error.load")}
          </p>
        ) : null}
        {saveError ? <p className="text-sm text-danger">{saveError}</p> : null}
        {saved ? (
          <p role="status" className="text-sm text-fg-muted">
            {t("budget.saved")}
          </p>
        ) : null}

        {stored ? (
          <div className="flex flex-wrap items-end gap-3">
            <label className="flex flex-col gap-1 text-sm text-fg">
              {t("budget.label")}
              <input
                type="number"
                min={1}
                max={stored.max_offers_per_run}
                value={value}
                onChange={(event) => {
                  setSaved(false);
                  setDraft(event.target.value);
                }}
                className="w-32 rounded-xl border border-border bg-surface px-3 py-2 text-fg"
              />
            </label>
            <Button
              type="button"
              disabled={!dirty || save.isPending}
              onClick={() => {
                setSaveError(null);
                setSaved(false);
                save.mutate(parsed);
              }}
            >
              {t("budget.save")}
            </Button>
            <p className="text-sm text-fg-muted">
              {t("budget.range", { max: stored.max_offers_per_run })}
            </p>
          </div>
        ) : null}
      </section>

      <section className="flex flex-col gap-2" aria-labelledby="ai-engine-limits-title">
        <h2 id="ai-engine-limits-title" className="font-display text-lg text-fg">
          {t("limits.title")}
        </h2>
        <p className="text-sm text-fg-muted">{t("limits.body")}</p>
        {stored ? (
          <table className="w-full max-w-xl border-collapse text-sm">
            <thead>
              <tr className="text-left text-fg-muted">
                <th scope="col" className="py-1 pr-4 font-medium">
                  {t("limits.task")}
                </th>
                <th scope="col" className="py-1 font-medium">
                  {t("limits.limits")}
                </th>
              </tr>
            </thead>
            <tbody>
              {stored.limits.map((limit) => (
                <tr key={limit.task} className="border-t border-border">
                  <th scope="row" className="py-1 pr-4 text-left font-normal text-fg">
                    {t(`onboarding_engine:tasks.name.${limit.task}`)}
                  </th>
                  <td className="py-1 text-fg-muted">
                    {t("limits.summary", {
                      turns: limit.max_turns,
                      toolCalls: limit.max_tool_calls,
                      seconds: limit.wall_clock_secs,
                    })}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : null}
      </section>

      <UsageSection chosen={engine.data?.chosen ?? null} t={t} tError={tError} />
    </section>
  );
}

/** The round-trip test call's usage (FR-3, PRD §6.3): what the CLI reported. */
function UsageSection({
  chosen,
  t,
  tError,
}: {
  chosen: { cli: string; model: string | null } | null;
  t: (key: string, options?: Record<string, unknown>) => string;
  tError: (key: string) => string;
}) {
  const [result, setResult] = useState<TestCallResult | null>(null);
  const [errorText, setErrorText] = useState<string | null>(null);

  const test = useMutation({
    mutationFn: () => commands.agentTestCall(chosen?.cli ?? "", chosen?.model ?? null).then(unwrap),
    onSuccess: (call) => {
      setResult(call);
      setErrorText(null);
    },
    onError: (error) => {
      setResult(null);
      setErrorText(isAppError(error) ? renderAppError(error, tError) : t("usage.error"));
    },
  });

  const reported =
    result !== null &&
    (result.tokens_in !== null || result.tokens_out !== null || result.cost_usd !== null);

  return (
    <section className="flex flex-col gap-2" aria-labelledby="ai-engine-usage-title">
      <h2 id="ai-engine-usage-title" className="font-display text-lg text-fg">
        {t("usage.title")}
      </h2>
      <p className="text-sm text-fg-muted">{t("usage.body")}</p>
      {chosen ? (
        <div className="flex items-center gap-3">
          <Button type="button" disabled={test.isPending} onClick={() => test.mutate()}>
            {test.isPending ? t("usage.running") : t("usage.run")}
          </Button>
        </div>
      ) : (
        <p className="text-sm text-fg-muted">{t("usage.noEngine")}</p>
      )}
      {errorText ? <p className="text-sm text-danger">{errorText}</p> : null}
      {result ? (
        <div className="flex flex-col gap-1 text-sm">
          {reported ? (
            <dl className="flex flex-wrap gap-x-6 gap-y-1">
              {result.tokens_in !== null ? (
                <div>
                  <dt className="text-fg-muted">{t("usage.tokensIn")}</dt>
                  <dd className="text-fg">{result.tokens_in}</dd>
                </div>
              ) : null}
              {result.tokens_out !== null ? (
                <div>
                  <dt className="text-fg-muted">{t("usage.tokensOut")}</dt>
                  <dd className="text-fg">{result.tokens_out}</dd>
                </div>
              ) : null}
              {result.cost_usd !== null ? (
                <div>
                  <dt className="text-fg-muted">{t("usage.cost")}</dt>
                  <dd className="text-fg">${result.cost_usd.toFixed(4)}</dd>
                </div>
              ) : null}
              <div>
                <dt className="text-fg-muted">{t("usage.latency")}</dt>
                <dd className="text-fg">{t("usage.latencyValue", { ms: result.latency_ms })}</dd>
              </div>
            </dl>
          ) : (
            <p className="text-fg-muted">{t("usage.notReported")}</p>
          )}
        </div>
      ) : null}
    </section>
  );
}
