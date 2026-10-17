import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { ClaimView, StudioDocument } from "@/lib/ipc/bindings";
import { cn } from "@/lib/utils";

/** Where an inline edit lands, as the core names the claim. */
export type EditSlot =
  | { slot: "summary" }
  | { slot: "cv_item"; section: number; item: number }
  | { slot: "cover_paragraph"; index: number };

export type StudioEditorProps = {
  /** The draft as the core read it back; the editor never edits it locally. */
  document: StudioDocument;
  /** Save an inline edit: the core stores a new version and re-runs the checks. */
  onEdit: (target: EditSlot, text: string) => void;
  /** Regenerate one section: `null` is the CV summary, a number a section or a
   * cover-letter paragraph. */
  onRegenerate: (section: number | null) => void;
  /** A request is in flight: the draft the user sees is no longer the newest. */
  busy?: boolean;
};

/**
 * The draft editor (S7.7, AC1, EXPERIENCE §9.5): the CV or cover letter claim by
 * claim, each with the Profile facts behind it, an inline edit, a regenerate
 * action per section, and the diff against the previous draft.
 *
 * The studio never renders a draft it edited itself: every edit is a new version
 * the core re-checks, and what comes back is what this shows. Agent-written text
 * is displayed as text, never as markup (ADR-010).
 */
export default function StudioEditor({
  document,
  onEdit,
  onRegenerate,
  busy = false,
}: StudioEditorProps) {
  const { t } = useTranslation("studio");
  const [editing, setEditing] = useState<{ key: string; text: string } | null>(null);
  const [diffOpen, setDiffOpen] = useState(false);

  const isCv = document.kind === "cv";
  const checks = document.checks;

  const claim = (
    slot: EditSlot,
    key: string,
    view: ClaimView,
    only?: { section: number | null; label: string },
  ) => (
    <li
      key={key}
      data-slot={slot.slot}
      data-backed={view.backed}
      className={cn(
        "flex flex-col gap-1 rounded-xl border-l-2 px-2 py-1",
        view.backed ? "border-l-transparent" : "border-l-warning bg-surface-muted",
      )}
    >
      {editing?.key === key ? (
        <form
          className="flex flex-col gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (editing.text.trim().length === 0) return;
            onEdit(slot, editing.text);
            setEditing(null);
          }}
        >
          <textarea
            aria-label={t("editor.text")}
            value={editing.text}
            rows={2}
            onChange={(event) => setEditing({ key, text: event.target.value })}
            className="w-full rounded-xl border border-border bg-surface p-2 text-sm text-fg"
          />
          <div className="flex gap-2">
            <button
              type="submit"
              disabled={busy || editing.text.trim().length === 0}
              className="rounded-xl bg-primary px-3 py-1 text-xs text-on-primary disabled:opacity-50"
            >
              {t("editor.save")}
            </button>
            <button
              type="button"
              onClick={() => setEditing(null)}
              className="rounded-xl px-3 py-1 text-xs text-fg-muted hover:text-fg"
            >
              {t("editor.cancel")}
            </button>
          </div>
        </form>
      ) : (
        <div className="flex items-start gap-2">
          <span className="min-w-0 flex-1 text-sm text-fg">{view.text}</span>
          {view.backed ? null : (
            <span className="shrink-0 rounded-xl bg-warning/10 px-2 py-0.5 text-xs text-warning">
              {view.unknown_facts.length > 0
                ? t("editor.unknown", { ids: view.unknown_facts.join(", ") })
                : t("editor.unbacked")}
            </span>
          )}
          {only ? regenerate(only.section, only.label) : null}
          <button
            type="button"
            disabled={busy}
            onClick={() => setEditing({ key, text: view.text })}
            className="shrink-0 text-xs text-primary hover:text-primary-hover disabled:opacity-50"
          >
            {t("editor.edit")}
          </button>
        </div>
      )}
      {view.facts.length > 0 ? (
        <p className="text-xs text-fg-subtle">
          {t("editor.facts", { labels: view.facts.map((fact) => fact.label).join(" · ") })}
        </p>
      ) : null}
    </li>
  );

  const regenerate = (section: number | null, label: string) => (
    <button
      type="button"
      disabled={busy}
      onClick={() => onRegenerate(section)}
      aria-label={t("editor.regenerate_label", { section: label })}
      className="shrink-0 text-xs text-primary hover:text-primary-hover disabled:opacity-50"
    >
      {t("editor.regenerate")}
    </button>
  );

  return (
    <section
      className="flex min-h-0 flex-col gap-3 rounded-2xl border border-border bg-surface p-4"
      aria-labelledby="studio-editor-title"
      data-kind={document.kind}
      data-ready={document.ready}
    >
      <div className="flex items-baseline justify-between gap-2">
        <h2 id="studio-editor-title" className="font-display text-lg text-fg">
          {isCv ? t("documents.cv") : t("documents.cover_letter")}
        </h2>
        <span className="text-xs text-fg-muted" data-version={document.version}>
          {t("documents.version", { version: document.version })}
        </span>
      </div>

      {document.language_fallback_from ? (
        <p className="rounded-xl bg-warning/10 px-3 py-2 text-xs text-warning" data-fallback="">
          {t("documents.fallback", { lang: document.language_fallback_from })}
        </p>
      ) : null}
      {document.untrusted ? (
        <p className="text-xs text-fg-subtle" data-untrusted="">
          {t("documents.untrusted")}
        </p>
      ) : null}

      {document.previous ? (
        <div className="flex items-center gap-2">
          <button
            type="button"
            aria-pressed={diffOpen}
            onClick={() => setDiffOpen(!diffOpen)}
            className="rounded-xl border border-border px-3 py-1 text-xs text-fg hover:text-primary"
          >
            {diffOpen ? t("diff.hide") : t("diff.show")}
          </button>
          <span className="text-xs text-fg-subtle">
            {t("diff.since", { version: document.previous.version })}
          </span>
        </div>
      ) : null}

      {diffOpen ? (
        <div className="flex flex-col gap-2 rounded-xl bg-surface-muted p-3" data-diff="">
          {document.diff.length === 0 ? (
            <p className="text-xs text-fg-muted">{t("diff.none")}</p>
          ) : (
            <ul className="flex flex-col gap-2">
              {document.diff.map((line) => (
                <li key={`${line.slot}:${line.before}:${line.after}`} data-change={line.change}>
                  <p className="text-xs text-fg-subtle">{line.slot}</p>
                  {line.before ? (
                    <p className="text-sm text-warning line-through">{line.before}</p>
                  ) : null}
                  {line.after ? <p className="text-sm text-fg">{line.after}</p> : null}
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}

      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto">
        {isCv ? (
          <>
            {document.text.summary ? (
              <div className="flex flex-col gap-1">
                <div className="flex items-center justify-between gap-2">
                  <h3 className="text-sm font-semibold text-fg">{t("editor.summary")}</h3>
                  {regenerate(null, t("editor.summary"))}
                </div>
                <ul>{claim({ slot: "summary" }, "summary", document.text.summary)}</ul>
              </div>
            ) : null}
            {document.text.sections.map((section, index) => (
              <div key={section.heading} className="flex flex-col gap-1">
                <div className="flex items-center justify-between gap-2">
                  <h3 className="text-sm font-semibold text-fg">{section.heading}</h3>
                  {regenerate(index, section.heading)}
                </div>
                <ul className="flex flex-col gap-1">
                  {section.items.map((item, itemIndex) =>
                    claim(
                      { slot: "cv_item", section: index, item: itemIndex },
                      `cv:${index}:${itemIndex}`,
                      item,
                    ),
                  )}
                </ul>
              </div>
            ))}
          </>
        ) : (
          <div className="flex flex-col gap-1">
            <h3 className="text-sm font-semibold text-fg">{t("editor.paragraphs")}</h3>
            <ul className="flex flex-col gap-1">
              {document.text.paragraphs.map((paragraph, index) =>
                claim({ slot: "cover_paragraph", index }, `cover:${index}`, paragraph, {
                  section: index,
                  label: t("editor.paragraph", { index: index + 1 }),
                }),
              )}
            </ul>
          </div>
        )}
      </div>

      {checks && checks.cut.length > 0 ? (
        <div className="rounded-xl bg-surface-muted p-3" data-cut="">
          <h3 className="text-sm font-semibold text-fg">{t("editor.cut.title")}</h3>
          <ul className="flex flex-col gap-1">
            {checks.cut.map((line) => (
              <li key={`${line.section}:${line.text}`} className="text-xs text-fg-muted">
                {t("editor.cut.line", { text: line.text })}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {checks && checks.layout.orphans > 0 ? (
        <p className="text-xs text-warning" data-orphans="">
          {t("editor.orphans", { count: checks.layout.orphans })}
        </p>
      ) : null}
    </section>
  );
}
