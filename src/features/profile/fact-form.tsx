import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { FactRevisionInput, FactView } from "@/lib/ipc/bindings";
import { AiDraft } from "./ai-draft";
import { ImproveFact } from "./improve";
import { takesEvidence } from "./sections";

/**
 * Add/edit form for one fact (AC1/AC2). Saving always creates a new version:
 * the form never edits a stored row, it sends the text the core appends as the
 * next revision.
 */
export function FactForm({
  kind,
  label,
  fact,
  saving,
  error,
  onSave,
  onCancel,
}: {
  kind: string;
  label: string;
  /** The fact being edited, `None` for a new item. */
  fact: FactView | null;
  saving: boolean;
  error: string | null;
  onSave: (input: FactRevisionInput) => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation("profile");
  const [textEn, setTextEn] = useState(fact?.text_en ?? "");
  const [textEs, setTextEs] = useState(fact?.text_es ?? "");
  const [evidence, setEvidence] = useState(fact?.evidence ?? "");
  const [missing, setMissing] = useState(false);

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (textEn.trim() === "" && textEs.trim() === "") {
      setMissing(true);
      return;
    }
    setMissing(false);
    onSave({
      item_id: fact?.item_id ?? null,
      kind,
      text_en: textEn.trim() || null,
      text_es: textEs.trim() || null,
      evidence: evidence.trim() || null,
      detail_json: fact?.detail_json ?? null,
    });
  };

  return (
    <form
      onSubmit={submit}
      aria-label={
        fact ? t("form.title.edit", { section: label }) : t("form.title.new", { section: label })
      }
      className="flex flex-col gap-3 rounded-2xl border border-border bg-surface p-4"
    >
      <p className="font-display text-lg text-fg">
        {fact ? t("form.title.edit", { section: label }) : t("form.title.new", { section: label })}
      </p>

      <label className="flex flex-col gap-1 text-sm text-fg-muted">
        {t("form.textEn")}
        <textarea
          value={textEn}
          onChange={(event) => setTextEn(event.target.value)}
          rows={2}
          className="rounded-xl border border-border bg-surface-muted px-3 py-2 text-fg"
        />
      </label>

      <label className="flex flex-col gap-1 text-sm text-fg-muted">
        {t("form.textEs")}
        <textarea
          value={textEs}
          onChange={(event) => setTextEs(event.target.value)}
          rows={2}
          className="rounded-xl border border-border bg-surface-muted px-3 py-2 text-fg"
        />
      </label>

      {/* FX8: a fact with one variant can draft the other, never auto-filled. */}
      {fact ? (
        <AiDraft
          fact={fact}
          onConfirmed={(lang, text) => {
            if (lang === "es") setTextEs(text);
            else setTextEn(text);
          }}
        />
      ) : null}

      {/* FX10b: an existing fact can be improved; the accepted text lands in
          the fields above so Save stores exactly what the user reviewed. */}
      {fact ? (
        <ImproveFact
          fact={fact}
          onConfirmed={(changed) => {
            if ("en" in changed) setTextEn(changed.en ?? "");
            if ("es" in changed) setTextEs(changed.es ?? "");
          }}
        />
      ) : null}

      {takesEvidence(kind) ? (
        <label className="flex flex-col gap-1 text-sm text-fg-muted">
          {t("form.evidence")}
          <input
            value={evidence}
            onChange={(event) => setEvidence(event.target.value)}
            className="rounded-xl border border-border bg-surface-muted px-3 py-2 text-fg"
          />
        </label>
      ) : null}

      {missing ? <p className="text-sm text-danger">{t("form.required")}</p> : null}
      {error ? <p className="text-sm text-danger">{error}</p> : null}

      <div className="flex items-center gap-2">
        <Button type="submit" size="sm" disabled={saving}>
          {t("form.save")}
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
          {t("form.cancel")}
        </Button>
      </div>
    </form>
  );
}
