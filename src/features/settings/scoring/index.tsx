import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { commands, type PreferencesSnapshot, type ScoringWeights } from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";
import SemanticMatch from "./embeddings";

/**
 * Settings → Scoring (S6.8, FR-58).
 *
 * The screen edits the weights the deterministic scorer reads — the eleven
 * factors and the two axis weights — and nothing else. The core owns the two
 * rules that matter (AD-11):
 *
 * * a value outside 0–100, or an axis with no weight at all, is refused, so
 *   the inputs here clamp and the save waits for a scorable document;
 * * a save is an ordinary Preferences change, so it lands as a **new prefs
 *   version** and marks the scores stale (FR-14). The next run re-scores with
 *   these weights and keys the new cards by that version (AC1).
 *
 * Reset writes the PRD's defaults — a change like any other, with its own
 * version, rather than a silent edit of the stored document.
 */
const DEFAULTS: ScoringWeights = {
  skills: 35,
  experience: 25,
  semantic: 15,
  location: 10,
  education: 10,
  language: 5,
  compensation: 30,
  company: 25,
  terms: 15,
  legitimacy: 20,
  alignment: 10,
  fit: 65,
  quality: 35,
};

const FIT_FIELDS: readonly (keyof ScoringWeights)[] = [
  "skills",
  "experience",
  "semantic",
  "location",
  "education",
  "language",
];

const QUALITY_FIELDS: readonly (keyof ScoringWeights)[] = [
  "compensation",
  "company",
  "terms",
  "legitimacy",
  "alignment",
];

const sum = (weights: ScoringWeights, fields: readonly (keyof ScoringWeights)[]) =>
  fields.reduce((total, field) => total + weights[field], 0);

const inRange = (weights: ScoringWeights) =>
  Object.values(weights).every((value) => Number.isInteger(value) && value >= 0 && value <= 100);

/** The two rules the core enforces, mirrored so Save never sends a refusal. */
function problem(weights: ScoringWeights): "range" | "axis" | null {
  if (!inRange(weights)) return "range";
  if (sum(weights, FIT_FIELDS) === 0 || sum(weights, QUALITY_FIELDS) === 0) return "axis";
  if (weights.fit === 0 && weights.quality === 0) return "axis";
  return null;
}

export default function SettingsScoring() {
  const { t } = useTranslation("settings_scoring");
  const { t: tError } = useTranslation();
  const queryClient = useQueryClient();
  const stored = useQuery({
    queryKey: ["preferences"],
    queryFn: () => commands.preferencesSnapshot().then(unwrap),
  });
  const [draft, setDraft] = useState<ScoringWeights | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  // A document written before S6.8 has no weights object: the core scores it
  // with the defaults, and so does this screen.
  const storedWeights = stored.data ? (stored.data.values.weights ?? DEFAULTS) : null;
  const values = draft ?? storedWeights ?? DEFAULTS;
  const invalid = problem(values);
  const dirty = storedWeights !== null && JSON.stringify(values) !== JSON.stringify(storedWeights);

  const save = useMutation({
    mutationFn: (weights: ScoringWeights) => {
      if (!stored.data) throw new Error("no preferences");
      return commands.preferencesSave({ ...stored.data.values, weights }).then(unwrap);
    },
    onSuccess: (snapshot: PreferencesSnapshot) => {
      queryClient.setQueryData(["preferences"], snapshot);
      setDraft(null);
      setSaved(true);
    },
    onError: (error) =>
      setSaveError(isAppError(error) ? renderAppError(error, tError) : t("error.save")),
  });

  const commit = (weights: ScoringWeights) => {
    setSaved(false);
    setSaveError(null);
    save.mutate(weights);
  };

  const update = (field: keyof ScoringWeights, raw: string) => {
    const parsed = Number.parseInt(raw, 10);
    if (!Number.isInteger(parsed)) return;
    setSaved(false);
    setDraft({ ...values, [field]: Math.min(100, Math.max(0, parsed)) });
  };

  return (
    <section className="flex flex-col gap-6 p-6" aria-labelledby="settings-scoring-title">
      <div className="flex items-center gap-2">
        <h1 id="settings-scoring-title" className="font-display text-2xl text-fg">
          {t("title")}
        </h1>
        {stored.data ? (
          <span className="rounded-xl bg-surface-muted px-2 py-1 text-xs text-fg-muted">
            {t("version", { version: stored.data.version })}
          </span>
        ) : null}
      </div>
      <p className="text-sm text-fg-muted">{t("intro")}</p>

      {/* FR-14: a change leaves the stored scores stale until the next re-score. */}
      {stored.data?.scores_stale ? (
        <p role="status" className="rounded-xl bg-surface-muted px-3 py-2 text-sm text-fg-muted">
          {t("stale")}
        </p>
      ) : null}
      {saved ? (
        <p role="status" className="text-sm text-fg-muted">
          {t("saved")}
        </p>
      ) : null}
      {saveError ? <p className="text-sm text-danger">{saveError}</p> : null}
      {stored.isPending ? <p className="text-sm text-fg-muted">{t("loading")}</p> : null}
      {stored.isError ? (
        <p className="text-sm text-danger">
          {isAppError(stored.error) ? renderAppError(stored.error, tError) : t("error.load")}
        </p>
      ) : null}

      <WeightGroup
        title={t("fit.title")}
        hint={t("fit.hint")}
        fields={FIT_FIELDS}
        weights={values}
        onChange={update}
        disabled={save.isPending || !stored.data}
      />
      <WeightGroup
        title={t("quality.title")}
        hint={t("quality.hint")}
        fields={QUALITY_FIELDS}
        weights={values}
        onChange={update}
        disabled={save.isPending || !stored.data}
      />

      <fieldset className="flex flex-col gap-2" aria-labelledby="scoring-axes-title">
        <legend id="scoring-axes-title" className="font-display text-lg text-fg">
          {t("axes.title")}
        </legend>
        <p className="text-sm text-fg-muted">{t("axes.hint")}</p>
        <div className="flex flex-wrap gap-4">
          {(["fit", "quality"] as const).map((axis) => (
            <label key={axis} className="flex flex-col gap-1 text-sm text-fg">
              {t(`settings_scoring:axes.${axis}`)}
              <input
                type="number"
                min={0}
                max={100}
                value={values[axis]}
                disabled={save.isPending || !stored.data}
                onChange={(event) => update(axis, event.target.value)}
                className="w-24 rounded-xl border border-border bg-surface px-3 py-2 text-fg"
              />
            </label>
          ))}
        </div>
      </fieldset>

      {invalid ? (
        <p role="alert" className="text-sm text-danger">
          {t(`settings_scoring:error.${invalid}`)}
        </p>
      ) : null}

      {/* S6.9: F3's model is a download, so it lives behind an explicit choice. */}
      <SemanticMatch />

      <div className="flex flex-wrap items-center gap-3">
        <Button
          type="button"
          disabled={!dirty || invalid !== null || save.isPending}
          onClick={() => commit(values)}
        >
          {t("save")}
        </Button>
        <Button
          type="button"
          variant="outline"
          disabled={
            !stored.data ||
            save.isPending ||
            JSON.stringify(storedWeights) === JSON.stringify(DEFAULTS)
          }
          onClick={() => commit(DEFAULTS)}
        >
          {t("reset")}
        </Button>
      </div>
    </section>
  );
}

/** One axis's factor weights, as a labelled row each (NFR-15: text, not colour). */
function WeightGroup({
  title,
  hint,
  fields,
  weights,
  onChange,
  disabled,
}: {
  title: string;
  hint: string;
  fields: readonly (keyof ScoringWeights)[];
  weights: ScoringWeights;
  onChange: (field: keyof ScoringWeights, value: string) => void;
  disabled: boolean;
}) {
  const { t } = useTranslation("settings_scoring");
  const id = title.replace(/\s+/g, "-").toLowerCase();
  return (
    <fieldset className="flex flex-col gap-2" aria-labelledby={`${id}-title`}>
      <legend id={`${id}-title`} className="font-display text-lg text-fg">
        {title}
      </legend>
      <p className="text-sm text-fg-muted">{hint}</p>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {fields.map((field) => (
          <label key={field} className="flex flex-col gap-1 text-sm text-fg">
            {t(`settings_scoring:factor.${field}`)}
            <input
              type="number"
              min={0}
              max={100}
              value={weights[field]}
              disabled={disabled}
              onChange={(event) => onChange(field, event.target.value)}
              className="w-24 rounded-xl border border-border bg-surface px-3 py-2 text-fg"
            />
          </label>
        ))}
      </div>
    </fieldset>
  );
}
