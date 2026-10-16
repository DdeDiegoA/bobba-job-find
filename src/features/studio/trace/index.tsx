import { useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";

/** A Profile fact behind a sentence, as the panel shows it (FR-73). */
export type TraceFact = {
  id: string;
  label: string;
};

/**
 * One claim of the draft, as the trace panel reads it (S7.3 / FR-73 / FR-74).
 * The props come from the validator's report joined with the Profile facts the
 * studio already holds; nothing here reads the store or re-renders the draft.
 */
export type TraceSentence = {
  text: string;
  /** The facts the sentence was built from. Empty = nothing backs it (AC1). */
  facts: TraceFact[];
  /** Fact ids the validator says the Profile does not have (FR-74). */
  unknown?: string[];
};

export type StudioTraceProps = {
  sentences: TraceSentence[];
  /** Fix one sentence: open it in the editor, or confirm the claim into the Profile. */
  onFix: (sentence: TraceSentence) => void;
};

/**
 * Claim traceability panel (AC2): every sentence of the draft with the Profile
 * facts behind it, shown on hover or focus. A sentence that no fact backs — or
 * that names facts the Profile does not have — is highlighted, and while any
 * such sentence exists *Ready* and *Send* stay blocked (AC1, FR-74).
 */
export default function StudioTrace({ sentences, onFix }: StudioTraceProps) {
  const { t } = useTranslation("studio_trace");
  const [open, setOpen] = useState<number | null>(null);

  const unbacked = (sentence: TraceSentence) =>
    sentence.facts.length === 0 || (sentence.unknown?.length ?? 0) > 0;
  const blocked = sentences.filter(unbacked).length;

  return (
    <section
      className="flex flex-col gap-3 rounded-2xl border border-border bg-surface p-4"
      aria-labelledby="studio-trace-title"
      data-blocked={blocked > 0}
    >
      <h2 id="studio-trace-title" className="font-display text-lg text-fg">
        {t("title")}
      </h2>
      <p className="text-xs text-fg-subtle">{t("hint")}</p>
      <p className="text-sm text-fg-muted" role="status">
        {blocked === 0 ? t("status.ready") : t("status.blocked", { count: blocked })}
      </p>

      <ul className="flex flex-col gap-2">
        {sentences.map((sentence, index) => {
          const flagged = unbacked(sentence);
          const expanded = open === index;
          return (
            <li
              // biome-ignore lint/suspicious/noArrayIndexKey: a sentence's identity is its position in the draft, which the validator slot also names
              key={index}
              data-trace={flagged ? "unbacked" : "backed"}
              className={cn(
                "flex flex-col gap-1 rounded-xl border-l-2 px-2 py-1",
                flagged ? "border-l-warning bg-surface-muted" : "border-l-transparent",
              )}
            >
              <div className="flex items-start gap-2">
                <button
                  type="button"
                  aria-expanded={expanded}
                  onClick={() => setOpen(expanded ? null : index)}
                  onMouseOver={() => setOpen(index)}
                  onMouseOut={() => setOpen(null)}
                  onFocus={() => setOpen(index)}
                  onBlur={() => setOpen(null)}
                  className="min-w-0 flex-1 text-left text-sm text-fg hover:text-primary"
                >
                  {sentence.text}
                </button>
                {flagged ? (
                  <>
                    <span className="shrink-0 rounded-xl bg-warning/10 px-2 py-0.5 text-xs text-warning">
                      {t("unlinked")}
                    </span>
                    <button
                      type="button"
                      onClick={() => onFix(sentence)}
                      className="shrink-0 text-xs text-primary hover:text-primary-hover"
                    >
                      {t("fix")}
                    </button>
                  </>
                ) : null}
              </div>

              {expanded ? (
                <div className="text-xs text-fg-muted">
                  {sentence.facts.length > 0 ? (
                    <>
                      <p>{t("sources")}</p>
                      <ul className="flex flex-col gap-1">
                        {sentence.facts.map((fact) => (
                          <li key={fact.id}>{fact.label}</li>
                        ))}
                      </ul>
                    </>
                  ) : null}
                  {(sentence.unknown?.length ?? 0) > 0 ? (
                    <p className="text-warning">
                      {t("unknown", { ids: sentence.unknown?.join(", ") })}
                    </p>
                  ) : null}
                </div>
              ) : null}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
