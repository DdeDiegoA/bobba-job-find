import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { AnswerView } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { useAnswers, useApproveAnswer } from "./use-answer-bank";

/**
 * The screening-answer approval surface (S7.8, surfaced by S9.4).
 *
 * A run drafts answers; the core stores them as untrusted rows and only the
 * user's approval makes one reusable (and fillable on another form). Editing
 * the text and approving is the same act: the answer the user saves is the one
 * the next form gets.
 */
export default function AnswerApprovePanel({ applicationId }: { applicationId: string }) {
  const { t } = useTranslation("apply_bar");
  const { t: tError } = useTranslation();
  const bank = useAnswers(applicationId);
  const approve = useApproveAnswer(applicationId);
  const [drafts, setDrafts] = useState<Record<string, string>>({});

  if (bank.isPending) return null;
  if (bank.isError) {
    const message = isAppError(bank.error)
      ? renderAppError(bank.error, tError)
      : tError("errors.unknown");
    return (
      <p role="alert" className="text-xs text-warning">
        {message}
      </p>
    );
  }

  const answers = bank.data.answers;
  const draftOf = (answer: AnswerView) => drafts[answer.id] ?? answer.answer ?? "";

  return (
    <div className="flex flex-col gap-2" data-answer-panel="">
      <h3 className="text-xs font-medium uppercase text-fg-muted">{t("answers.title")}</h3>
      {answers.length === 0 ? (
        <p className="text-xs text-fg-muted">{t("answers.empty")}</p>
      ) : (
        <ul className="flex flex-col gap-3 text-sm">
          {answers.map((answer) => (
            <li key={answer.id} className="flex flex-col gap-1" data-answer={answer.id}>
              <div className="flex flex-wrap items-baseline justify-between gap-2">
                <span className="text-fg-muted">{answer.question}</span>
                <span className="text-xs text-fg-muted" data-approved={answer.approved}>
                  {answer.approved ? t("answers.approved") : t("answers.draft")}
                </span>
              </div>
              <textarea
                className="min-h-16 w-full rounded-xl border border-border bg-surface p-2 text-sm text-fg"
                aria-label={answer.question}
                value={draftOf(answer)}
                onChange={(event) =>
                  setDrafts((current) => ({ ...current, [answer.id]: event.target.value }))
                }
              />
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  variant="outline"
                  disabled={
                    answer.approved || approve.isPending || draftOf(answer).trim().length === 0
                  }
                  onClick={() =>
                    approve.mutate({
                      answerId: answer.id,
                      // `null` confirms it as it stands; edited text is saved as given.
                      answer: draftOf(answer) === (answer.answer ?? "") ? null : draftOf(answer),
                    })
                  }
                >
                  {t("answers.save")}
                </Button>
                <span className="text-xs text-fg-muted">{t("answers.edit_hint")}</span>
              </div>
            </li>
          ))}
        </ul>
      )}
      {approve.isError ? (
        <p role="alert" className="text-xs text-warning">
          {isAppError(approve.error) ? renderAppError(approve.error, tError) : t("answers.failed")}
        </p>
      ) : null}
    </div>
  );
}
