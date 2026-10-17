import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { commands, type DecisionView, type RuleView } from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";
import { DecisionReasons, minuteClock } from "./reasons";
import { RuleForm } from "./rule-form";
import {
  useDeleteRule,
  useExplain,
  useResetRules,
  useRules,
  useSaveRule,
  useSetAuto,
  useSetGlobalCap,
} from "./use-rules";

/** The application list the "why" check offers, read from the Tracker. */
function useApplications() {
  return useQuery({
    queryKey: ["apply", "rules", "applications"],
    queryFn: () => commands.trackerBoard().then(unwrap),
  });
}

/** The reason text an error carries (CH7 keys resolve in `common`). */
function errorText(error: unknown, fallback: string, t: (key: string) => string): string {
  return isAppError(error) ? renderAppError(error, t) : fallback;
}

/**
 * Settings → Applying (FR-83, FR-87, FR-108, EXPERIENCE §9.12).
 *
 * The screen edits rules and switches Auto on; it never decides a send. The
 * "Check" panel calls the same `decide()` the send paths call, so the reasons
 * shown here are the ones enforcement uses — verbatim, in the order the core
 * gave them (AC2).
 */
export default function SettingsApplyingRules() {
  const { t } = useTranslation("applying");
  const { t: tError } = useTranslation();
  const rules = useRules();
  const save = useSaveRule();
  const remove = useDeleteRule();
  const setAuto = useSetAuto();
  const reset = useResetRules();
  const setCap = useSetGlobalCap();
  const explain = useExplain();

  const [editing, setEditing] = useState<RuleView | null>(null);
  const [creating, setCreating] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [understood, setUnderstood] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [capDraft, setCapDraft] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const snapshot = rules.data;
  const applications = useApplications();
  const [applicationId, setApplicationId] = useState("");
  const [channel, setChannel] = useState("email");
  const [decision, setDecision] = useState<DecisionView | null>(null);

  const cards =
    applications.data?.columns.flatMap((column) =>
      column.cards.map((card) => ({ ...card, stage: column.stage })),
    ) ?? [];

  const fail = (fallback: string) => (cause: unknown) =>
    setError(errorText(cause, fallback, tError));

  const closeForm = () => {
    setCreating(false);
    setEditing(null);
  };

  return (
    <section className="flex flex-col gap-6 p-6" aria-labelledby="settings-applying-title">
      <h1 id="settings-applying-title" className="font-display text-2xl text-fg">
        {t("title")}
      </h1>

      {rules.isPending ? <p className="text-sm text-fg-muted">{t("loading")}</p> : null}
      {rules.isError ? <p className="text-sm text-danger">{t("error.load")}</p> : null}
      {error ? <p className="text-sm text-danger">{error}</p> : null}

      {snapshot ? (
        <>
          <section className="flex flex-col gap-2 rounded-2xl border border-border p-4">
            <h2 className="font-medium text-fg">{t("mode.title")}</h2>
            <p className="text-sm text-fg-muted">
              {snapshot.auto_enabled ? t("mode.on_hint") : t("mode.off_hint")}
            </p>
            <div className="flex items-center gap-2">
              <span
                className={
                  snapshot.auto_enabled
                    ? "rounded-xl bg-primary-soft px-2 py-1 text-xs text-on-primary-soft"
                    : "rounded-xl bg-surface-muted px-2 py-1 text-xs text-fg-muted"
                }
              >
                {snapshot.auto_enabled ? t("mode.auto") : t("mode.review")}
              </span>
              {snapshot.auto_enabled ? (
                <Button
                  type="button"
                  variant="ghost"
                  disabled={setAuto.isPending}
                  onClick={() =>
                    setAuto.mutate(
                      { enabled: false, confirmed: false },
                      {
                        onError: fail(t("error.enable")),
                      },
                    )
                  }
                >
                  {t("mode.disable")}
                </Button>
              ) : (
                <Button
                  type="button"
                  disabled={setAuto.isPending || snapshot.rules.length === 0}
                  onClick={() => {
                    setUnderstood(false);
                    setConfirming(true);
                  }}
                >
                  {t("mode.enable")}
                </Button>
              )}
            </div>
            {snapshot.rules.length === 0 ? (
              <p className="text-xs text-fg-muted">{t("mode.needs_rule")}</p>
            ) : null}

            {confirming ? (
              <div
                role="dialog"
                aria-modal="true"
                aria-labelledby="auto-confirm-title"
                className="flex flex-col gap-3 rounded-2xl border border-border bg-surface-muted p-4"
              >
                <h3 id="auto-confirm-title" className="font-medium text-fg">
                  {t("confirm.title")}
                </h3>
                <p className="text-sm text-fg-muted">{t("confirm.body")}</p>
                <p className="text-sm text-fg-muted">
                  {t("confirm.caps", { cap: snapshot.global_daily_cap })}
                </p>
                <p className="text-sm text-fg-muted">{t("confirm.review_first")}</p>
                <label className="flex items-center gap-2 text-sm text-fg">
                  <input
                    type="checkbox"
                    checked={understood}
                    onChange={(event) => setUnderstood(event.target.checked)}
                  />
                  {t("confirm.accept")}
                </label>
                <div className="flex gap-2">
                  <Button
                    type="button"
                    disabled={!understood || setAuto.isPending}
                    onClick={() =>
                      setAuto.mutate(
                        { enabled: true, confirmed: understood },
                        {
                          onSuccess: () => setConfirming(false),
                          onError: fail(t("error.enable")),
                        },
                      )
                    }
                  >
                    {t("confirm.confirm")}
                  </Button>
                  <Button type="button" variant="ghost" onClick={() => setConfirming(false)}>
                    {t("confirm.cancel")}
                  </Button>
                </div>
              </div>
            ) : null}
          </section>

          <section className="flex flex-col gap-3">
            <div className="flex items-center justify-between">
              <h2 className="font-medium text-fg">{t("rules.title")}</h2>
              <Button
                type="button"
                onClick={() => {
                  setEditing(null);
                  setCreating(true);
                }}
              >
                {t("rules.new")}
              </Button>
            </div>

            {snapshot.rules.length === 0 ? (
              <EmptyState title={t("rules.title")} description={t("rules.empty")} />
            ) : null}

            <ul className="flex flex-col gap-2">
              {snapshot.rules.map((rule) => (
                <li
                  key={rule.id}
                  className="flex flex-col gap-1 rounded-2xl border border-border p-3"
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-medium text-fg">{rule.name}</span>
                    <span className="text-xs text-fg-muted">
                      {rule.enabled ? t("rules.on") : t("rules.off")}
                    </span>
                  </div>
                  <span className="text-sm text-fg-muted">
                    {t("rules.scope", {
                      sources:
                        rule.source_ids.length === 0
                          ? t("rules.scope_all")
                          : rule.source_ids.join(", "),
                    })}
                  </span>
                  <span className="text-sm text-fg-muted">
                    {t("rules.thresholds", {
                      rank: Math.round(rule.min_rank ?? 0),
                      fit: Math.round(rule.min_fit ?? 0),
                      confidence: t(`applying:form.confidence.${rule.min_confidence}`),
                    })}
                  </span>
                  <span className="text-sm text-fg-muted">
                    {t("rules.cap", {
                      daily_cap: rule.daily_cap,
                      per_company_cap: rule.per_company_cap,
                      per_company_days: rule.per_company_days,
                    })}
                  </span>
                  <span className="text-sm text-fg-muted">
                    {t("rules.hours", {
                      start: minuteClock(rule.active_start_minute),
                      end: minuteClock(rule.active_end_minute),
                    })}
                  </span>
                  <div className="flex gap-2 pt-1">
                    <Button
                      type="button"
                      variant="ghost"
                      onClick={() => {
                        setCreating(false);
                        setEditing(rule);
                      }}
                    >
                      {t("rules.edit")}
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      disabled={remove.isPending}
                      onClick={() => remove.mutate(rule.id, { onError: fail(t("error.delete")) })}
                    >
                      {t("rules.delete")}
                    </Button>
                  </div>
                </li>
              ))}
            </ul>

            {creating || editing ? (
              <RuleForm
                key={editing?.id ?? "new"}
                initial={editing}
                saving={save.isPending}
                onCancel={closeForm}
                onSave={(input) =>
                  save.mutate(input, {
                    onSuccess: () => {
                      setError(null);
                      closeForm();
                    },
                    onError: fail(t("error.save")),
                  })
                }
              />
            ) : null}
          </section>

          <section className="flex flex-col gap-3 rounded-2xl border border-border p-4">
            <h2 className="font-medium text-fg">{t("settings.title")}</h2>
            <p className="text-sm text-fg-muted">
              {t("settings.spacing", {
                min: snapshot.spacing_min_minutes,
                max: snapshot.spacing_max_minutes,
              })}
            </p>
            <div className="flex flex-wrap items-end gap-2">
              <label className="flex flex-col gap-1 text-sm text-fg-muted">
                {t("settings.global_cap")}
                <input
                  className="rounded-xl border border-border bg-surface-muted px-3 py-2 text-fg"
                  type="number"
                  min={1}
                  max={25}
                  value={capDraft ?? String(snapshot.global_daily_cap)}
                  onChange={(event) => setCapDraft(event.target.value)}
                />
              </label>
              <Button
                type="button"
                disabled={capDraft === null || setCap.isPending}
                onClick={() =>
                  setCap.mutate(Number(capDraft), {
                    onSuccess: () => {
                      setCapDraft(null);
                      setError(null);
                    },
                    onError: fail(t("error.cap")),
                  })
                }
              >
                {t("settings.save_cap")}
              </Button>
            </div>

            {resetting ? (
              <div className="flex flex-col gap-2">
                <p className="text-sm text-fg-muted">{t("settings.reset_confirm")}</p>
                <div className="flex gap-2">
                  <Button
                    type="button"
                    disabled={reset.isPending}
                    onClick={() =>
                      reset.mutate(undefined, {
                        onSuccess: () => {
                          setResetting(false);
                          setError(null);
                        },
                        onError: fail(t("error.reset")),
                      })
                    }
                  >
                    {t("settings.reset_yes")}
                  </Button>
                  <Button type="button" variant="ghost" onClick={() => setResetting(false)}>
                    {t("settings.reset_no")}
                  </Button>
                </div>
              </div>
            ) : (
              <Button type="button" variant="ghost" onClick={() => setResetting(true)}>
                {t("settings.reset")}
              </Button>
            )}
          </section>

          <section className="flex flex-col gap-3 rounded-2xl border border-border p-4">
            <h2 className="font-medium text-fg">{t("check.title")}</h2>
            <p className="text-sm text-fg-muted">{t("check.hint")}</p>
            {cards.length === 0 ? (
              <p className="text-sm text-fg-muted">{t("check.none")}</p>
            ) : (
              <div className="flex flex-wrap items-end gap-2">
                <label className="flex flex-col gap-1 text-sm text-fg-muted">
                  {t("check.application")}
                  <select
                    className="rounded-xl border border-border bg-surface-muted px-3 py-2 text-fg"
                    value={applicationId}
                    onChange={(event) => setApplicationId(event.target.value)}
                  >
                    <option value="">{t("check.choose")}</option>
                    {cards.map((card) => (
                      <option key={card.id} value={card.id}>
                        {card.title} · {card.company}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="flex flex-col gap-1 text-sm text-fg-muted">
                  {t("check.channel")}
                  <select
                    className="rounded-xl border border-border bg-surface-muted px-3 py-2 text-fg"
                    value={channel}
                    onChange={(event) => setChannel(event.target.value)}
                  >
                    <option value="email">{t("applying:form.channel.email")}</option>
                    <option value="ats_form">{t("applying:form.channel.ats_form")}</option>
                    <option value="sanctioned_api">
                      {t("applying:form.channel.sanctioned_api")}
                    </option>
                  </select>
                </label>
                <Button
                  type="button"
                  disabled={applicationId === "" || explain.isPending}
                  onClick={() => {
                    setError(null);
                    setDecision(null);
                    explain.mutate(
                      {
                        applicationId,
                        channel,
                        localMinute: new Date().getHours() * 60 + new Date().getMinutes(),
                      },
                      {
                        onSuccess: setDecision,
                        onError: fail(t("error.explain")),
                      },
                    );
                  }}
                >
                  {t("check.run")}
                </Button>
              </div>
            )}

            {decision ? (
              <div role="status" className="flex flex-col gap-2">
                <p className="text-sm text-fg">
                  {decision.kind === "auto"
                    ? t("check.auto", { rule: decision.rule_id ?? "" })
                    : decision.kind === "review"
                      ? t("check.review")
                      : t("check.block")}
                </p>
                {decision.reasons.length > 0 ? (
                  <DecisionReasons reasons={decision.reasons} />
                ) : null}
              </div>
            ) : null}
          </section>
        </>
      ) : null}
    </section>
  );
}
