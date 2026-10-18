import { useState } from "react";
import { Button } from "@/components/ui/button";
import type { FactEdit, ProposalView } from "@/lib/ipc/bindings";

/**
 * The one review card every AI proposal goes through (FX8, S3.6 AC3).
 *
 * A proposal is shown **verbatim as plain text** — whitespace preserved, never
 * truncated, never rendered as markup (AD-7, AD-15) — and nothing is saved
 * until the user confirms it or sends their own edit. The onboarding gap
 * interview and the Profile's language draft render the same card, so the
 * "LLM proposes, the core disposes" rule looks the same everywhere.
 *
 * Labels are passed in because the two surfaces own different catalogs
 * (`onboarding_gaps` and `profile`); the card itself names no namespace.
 */

/** Every string the card renders, already translated by the caller. */
export interface ProposalReviewLabels {
  /** Heading for the proposal's section (e.g. "Experience"). */
  kindLabel: string;
  textEn: string;
  textEs: string;
  confirm: string;
  edit: string;
  saveEdit: string;
  cancel: string;
  discard: string;
  editHint: string;
  untrusted: string;
}

export function ProposalReviewCard({
  proposal,
  busy,
  labels,
  onConfirm,
  onSaveEdit,
  onDiscard,
}: {
  proposal: ProposalView;
  busy: boolean;
  labels: ProposalReviewLabels;
  onConfirm: () => void;
  /** The user's own edit; the core saves exactly this. */
  onSaveEdit: (edit: FactEdit) => void;
  onDiscard: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [textEn, setTextEn] = useState(proposal.text_en ?? "");
  const [textEs, setTextEs] = useState(proposal.text_es ?? "");

  const id = `proposal-${proposal.id}`;

  if (editing) {
    return (
      <li className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-4">
        <h3 className="font-medium text-fg">{labels.kindLabel}</h3>
        <label htmlFor={`${id}-en`} className="text-sm font-medium text-fg">
          {labels.textEn}
        </label>
        <textarea
          id={`${id}-en`}
          className="min-h-20 rounded-xl border border-border bg-surface p-3 text-fg"
          value={textEn}
          onChange={(event) => setTextEn(event.target.value)}
        />
        <label htmlFor={`${id}-es`} className="text-sm font-medium text-fg">
          {labels.textEs}
        </label>
        <textarea
          id={`${id}-es`}
          className="min-h-20 rounded-xl border border-border bg-surface p-3 text-fg"
          value={textEs}
          onChange={(event) => setTextEs(event.target.value)}
        />
        <p className="text-sm text-fg-muted">{labels.editHint}</p>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            disabled={busy}
            onClick={() =>
              onSaveEdit({
                text_en: textEn.trim() === "" ? null : textEn,
                text_es: textEs.trim() === "" ? null : textEs,
                evidence: proposal.evidence,
                detail_json: proposal.detail_json,
              })
            }
          >
            {labels.saveEdit}
          </Button>
          <Button type="button" variant="ghost" onClick={() => setEditing(false)}>
            {labels.cancel}
          </Button>
        </div>
      </li>
    );
  }

  return (
    <li className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-4">
      <h3 className="font-medium text-fg">{labels.kindLabel}</h3>
      {proposal.untrusted_origin ? (
        <p className="text-sm text-warning">{labels.untrusted}</p>
      ) : null}
      {proposal.text_en ? (
        <div className="flex flex-col gap-1">
          <span className="text-xs uppercase tracking-wide text-fg-subtle">{labels.textEn}</span>
          <p className="whitespace-pre-wrap break-words text-fg">{proposal.text_en}</p>
        </div>
      ) : null}
      {proposal.text_es ? (
        <div className="flex flex-col gap-1">
          <span className="text-xs uppercase tracking-wide text-fg-subtle">{labels.textEs}</span>
          <p className="whitespace-pre-wrap break-words text-fg">{proposal.text_es}</p>
        </div>
      ) : null}
      <div className="flex flex-wrap gap-2">
        <Button type="button" disabled={busy} onClick={onConfirm}>
          {labels.confirm}
        </Button>
        <Button type="button" variant="outline" disabled={busy} onClick={() => setEditing(true)}>
          {labels.edit}
        </Button>
        <Button type="button" variant="ghost" disabled={busy} onClick={onDiscard}>
          {labels.discard}
        </Button>
      </div>
    </li>
  );
}
