import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { RuleInput, RuleView } from "@/lib/ipc/bindings";
import { minuteClock } from "./reasons";

const fieldClass = "rounded-xl border border-border bg-surface-muted px-3 py-2 text-fg";
const labelClass = "flex flex-col gap-1 text-sm text-fg-muted";

/** The channels a rule may target (FR-83). Session Board is not selectable yet. */
const CHANNELS = [
  { value: "email", labelKey: "applying:form.channel.email" },
  { value: "ats_form", labelKey: "applying:form.channel.ats_form" },
  { value: "sanctioned_api", labelKey: "applying:form.channel.sanctioned_api" },
  { value: "session_board", labelKey: "applying:form.channel.session_board" },
] as const;

const CONFIDENCES = [
  { value: "low", labelKey: "applying:form.confidence.low" },
  { value: "medium", labelKey: "applying:form.confidence.medium" },
  { value: "high", labelKey: "applying:form.confidence.high" },
] as const;

const VERDICTS = [
  { value: "strong", labelKey: "applying:form.verdict.strong" },
  { value: "good", labelKey: "applying:form.verdict.good" },
  { value: "moderate", labelKey: "applying:form.verdict.moderate" },
  { value: "weak", labelKey: "applying:form.verdict.weak" },
  { value: "poor", labelKey: "applying:form.verdict.poor" },
] as const;

/** `HH:MM` from minutes of the day, for a time input's value. */
function clock(value: number): string {
  return minuteClock(value);
}

/** Minutes of the day from an `HH:MM` time input, `NaN` for an empty one. */
function minutes(value: string): number {
  const [hours, mins] = value.split(":").map(Number);
  if (Number.isNaN(hours) || Number.isNaN(mins)) return Number.NaN;
  return hours * 60 + mins;
}

/** A new rule's draft: the same conservative defaults core compiles in (FR-83). */
const draftDefaults = {
  name: "",
  sources: "",
  channels: ["email"] as string[],
  minRank: "80",
  minFit: "80",
  minConfidence: "medium",
  allowFlags: false,
  minVerdict: "good",
  dailyCap: "5",
  perCompanyCap: "1",
  perCompanyDays: "30",
  activeFrom: "09:00",
  activeTo: "18:00",
};

/**
 * The Auto Rule editor (AC1): the fields FR-83 names, validated by the core
 * when saved (AD-11). The screen only proposes — every bound (cap 1–25, rank
 * 0–100, a real window) is refused there if this form lets it through.
 */
export function RuleForm({
  initial,
  saving,
  onSave,
  onCancel,
}: {
  initial: RuleView | null;
  saving: boolean;
  onSave: (input: RuleInput) => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation("applying");
  const [draft, setDraft] = useState(() =>
    initial
      ? {
          name: initial.name,
          sources: initial.source_ids.join("\n"),
          channels: initial.channels,
          minRank: String(initial.min_rank ?? 0),
          minFit: String(initial.min_fit ?? 0),
          minConfidence: initial.min_confidence,
          allowFlags: initial.allow_flags,
          minVerdict: initial.min_verdict,
          dailyCap: String(initial.daily_cap),
          perCompanyCap: String(initial.per_company_cap),
          perCompanyDays: String(initial.per_company_days),
          activeFrom: clock(initial.active_start_minute),
          activeTo: clock(initial.active_end_minute),
          enabled: initial.enabled,
        }
      : { ...draftDefaults, enabled: true },
  );

  const set = <K extends keyof typeof draft>(key: K, value: (typeof draft)[K]) =>
    setDraft((current) => ({ ...current, [key]: value }));

  const toggleChannel = (channel: string) => {
    const next = draft.channels.includes(channel)
      ? draft.channels.filter((value) => value !== channel)
      : [...draft.channels, channel];
    set("channels", next);
  };

  const submit = () => {
    const sources = draft.sources
      .split(/[\n,]/)
      .map((source) => source.trim())
      .filter((source) => source.length > 0);
    onSave({
      id: initial?.id ?? null,
      name: draft.name,
      enabled: draft.enabled,
      source_ids: sources,
      channels: draft.channels,
      min_rank: Number(draft.minRank),
      min_fit: Number(draft.minFit),
      min_confidence: draft.minConfidence,
      allow_flags: draft.allowFlags,
      min_verdict: draft.minVerdict,
      daily_cap: Number(draft.dailyCap),
      per_company_cap: Number(draft.perCompanyCap),
      per_company_days: Number(draft.perCompanyDays),
      active_start_minute: minutes(draft.activeFrom),
      active_end_minute: minutes(draft.activeTo),
    });
  };

  // Session Board only becomes selectable once that Source's Auto opt-in
  // lands (S11.3); a rule that already targets it keeps it, so an edit cannot
  // silently widen Auto to a channel the user chose earlier.
  const sessionBoardFixed = initial?.channels.includes("session_board") ?? false;

  return (
    <form
      className="flex flex-col gap-4 rounded-2xl border border-border p-4"
      aria-label={t("form.title")}
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >
      <label className={labelClass}>
        {t("form.name")}
        <input
          className={fieldClass}
          value={draft.name}
          onChange={(event) => set("name", event.target.value)}
          required
        />
      </label>

      <label className={labelClass}>
        {t("form.sources")}
        <textarea
          className={fieldClass}
          rows={2}
          value={draft.sources}
          placeholder={t("form.sources_placeholder")}
          onChange={(event) => set("sources", event.target.value)}
        />
        <span className="text-xs">{t("form.sources_hint")}</span>
      </label>

      <fieldset className="flex flex-wrap gap-4">
        <legend className="mb-1 text-sm text-fg-muted">{t("form.channels")}</legend>
        {CHANNELS.map((channel) => {
          const fixed = channel.value === "session_board";
          return (
            <label key={channel.value} className="flex items-center gap-2 text-sm text-fg">
              <input
                type="checkbox"
                checked={draft.channels.includes(channel.value)}
                disabled={fixed && !sessionBoardFixed}
                onChange={() => toggleChannel(channel.value)}
              />
              {t(channel.labelKey)}
            </label>
          );
        })}
        <span className="w-full text-xs text-fg-muted">{t("form.session_board_hint")}</span>
      </fieldset>

      <div className="flex flex-wrap gap-4">
        <label className={labelClass}>
          {t("form.min_rank")}
          <input
            className={fieldClass}
            type="number"
            min={0}
            max={100}
            value={draft.minRank}
            onChange={(event) => set("minRank", event.target.value)}
          />
        </label>
        <label className={labelClass}>
          {t("form.min_fit")}
          <input
            className={fieldClass}
            type="number"
            min={0}
            max={100}
            value={draft.minFit}
            onChange={(event) => set("minFit", event.target.value)}
          />
        </label>
        <label className={labelClass}>
          {t("form.min_confidence")}
          <select
            className={fieldClass}
            value={draft.minConfidence}
            onChange={(event) => set("minConfidence", event.target.value)}
          >
            {CONFIDENCES.map((option) => (
              <option key={option.value} value={option.value}>
                {t(option.labelKey)}
              </option>
            ))}
          </select>
        </label>
        <label className={labelClass}>
          {t("form.min_verdict")}
          <select
            className={fieldClass}
            value={draft.minVerdict}
            onChange={(event) => set("minVerdict", event.target.value)}
          >
            {VERDICTS.map((option) => (
              <option key={option.value} value={option.value}>
                {t(option.labelKey)}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-2 self-end pb-2 text-sm text-fg">
          <input
            type="checkbox"
            checked={draft.allowFlags}
            onChange={(event) => set("allowFlags", event.target.checked)}
          />
          {t("form.allow_flags")}
        </label>
      </div>

      <div className="flex flex-wrap gap-4">
        <label className={labelClass}>
          {t("form.daily_cap")}
          <input
            className={fieldClass}
            type="number"
            min={1}
            max={25}
            value={draft.dailyCap}
            onChange={(event) => set("dailyCap", event.target.value)}
          />
        </label>
        <label className={labelClass}>
          {t("form.per_company_cap")}
          <input
            className={fieldClass}
            type="number"
            min={1}
            value={draft.perCompanyCap}
            onChange={(event) => set("perCompanyCap", event.target.value)}
          />
        </label>
        <label className={labelClass}>
          {t("form.per_company_days")}
          <input
            className={fieldClass}
            type="number"
            min={1}
            max={365}
            value={draft.perCompanyDays}
            onChange={(event) => set("perCompanyDays", event.target.value)}
          />
        </label>
        <label className={labelClass}>
          {t("form.active_from")}
          <input
            className={fieldClass}
            type="time"
            value={draft.activeFrom}
            onChange={(event) => set("activeFrom", event.target.value)}
          />
        </label>
        <label className={labelClass}>
          {t("form.active_to")}
          <input
            className={fieldClass}
            type="time"
            value={draft.activeTo}
            onChange={(event) => set("activeTo", event.target.value)}
          />
        </label>
      </div>

      <div className="flex gap-2">
        <Button type="submit" disabled={saving}>
          {t("form.save")}
        </Button>
        <Button type="button" variant="ghost" onClick={onCancel}>
          {t("form.cancel")}
        </Button>
      </div>
    </form>
  );
}
