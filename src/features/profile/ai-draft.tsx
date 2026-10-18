import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { Button, buttonVariants } from "@/components/ui/button";
import { ProposalReviewCard } from "@/features/proposal-review";
import { useCapability } from "@/lib/capabilities";
import type { FactEdit, FactView, ProposalView } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { useConfirmProposal, useDiscardProposal, useDraftLanguage } from "./use-profile";

/** Where the user picks or changes the Agent CLI (FX7's Settings page). */
const AI_ENGINE_PATH = "/settings/ai-engine";

/**
 * Draft the language a fact is missing (FX8, issue 5).
 *
 * The draft is written by the `profile_gaps` agent run — the same runtime and
 * lockdown gate onboarding uses — and it arrives as a **pending proposal**: the
 * user sees it verbatim, may edit it, and nothing is saved until they confirm.
 * In Limited mode the action is disabled with the standard "no engine" copy and
 * a way to connect one (FX8 AC4).
 */
export function AiDraft({
  fact,
  onConfirmed,
}: {
  fact: FactView;
  /** The confirmed text, so the form's own field matches the saved version. */
  onConfirmed: (lang: string, text: string) => void;
}) {
  const { t } = useTranslation("profile");
  const { t: tError } = useTranslation();
  const { t: tGated } = useTranslation("gated");
  const access = useCapability("profile_translation");
  const draft = useDraftLanguage();
  const confirm = useConfirmProposal();
  const discard = useDiscardProposal();
  const [pending, setPending] = useState<ProposalView[]>([]);
  const [errorText, setErrorText] = useState<string | null>(null);
  // The fact prop is the snapshot from when the form opened: once the user
  // confirms, the gap is closed even though this object still says otherwise.
  const [filled, setFilled] = useState(false);

  const missing = fact.missing_variant;
  if (missing === null || filled) return null;

  const language = missing === "es" ? t("form.textEs") : t("form.textEn");
  const fail = (error: unknown) =>
    setErrorText(isAppError(error) ? renderAppError(error, tError) : tError("errors.unknown"));

  const confirmed = (proposal: ProposalView, edit: FactEdit | null, lang: string): string => {
    if (edit) return (lang === "es" ? edit.text_es : edit.text_en) ?? "";
    return (lang === "es" ? proposal.text_es : proposal.text_en) ?? "";
  };

  const noticeKey =
    access.reason === "cli_unsupported"
      ? { title: "notice.unsupportedTitle", body: "notice.unsupportedBody" }
      : { title: "notice.limitedTitle", body: "notice.limitedBody" };

  return (
    <section aria-label={t("drafts.aria")} className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <p className="text-xs text-warning">{t(`fact.missing.${missing}`)}</p>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={!access.available || draft.isPending}
          onClick={() => {
            setErrorText(null);
            draft.mutate(
              { itemId: fact.item_id, lang: missing },
              {
                onSuccess: (outcome) => setPending(outcome.proposals),
                onError: fail,
              },
            );
          }}
        >
          {draft.isPending ? t("drafts.drafting") : t("drafts.draft", { lang: language })}
        </Button>
      </div>

      {!access.available && access.reason ? (
        <div
          className="flex flex-col items-start gap-1 rounded-xl bg-surface-muted p-3 text-sm"
          role="status"
        >
          <p className="font-medium text-fg">{tGated(noticeKey.title)}</p>
          <p className="text-fg-muted">{tGated(noticeKey.body)}</p>
          <Link to={AI_ENGINE_PATH} className={buttonVariants({ variant: "outline", size: "sm" })}>
            {t("drafts.openSettings")}
          </Link>
        </div>
      ) : null}

      {errorText ? <p className="text-sm text-danger">{errorText}</p> : null}

      {pending.length > 0 ? (
        <ul className="flex flex-col gap-3">
          {pending.map((proposal) => (
            <ProposalReviewCard
              key={proposal.id}
              proposal={proposal}
              busy={confirm.isPending || discard.isPending}
              labels={{
                kindLabel: t(`sections.${proposal.kind}`),
                textEn: t("form.textEn"),
                textEs: t("form.textEs"),
                confirm: t("drafts.confirm"),
                edit: t("drafts.edit"),
                saveEdit: t("drafts.saveEdit"),
                cancel: t("drafts.cancel"),
                discard: t("drafts.discard"),
                editHint: t("drafts.editHint"),
                untrusted: t("drafts.untrusted"),
              }}
              onConfirm={() =>
                confirm.mutate(
                  { id: proposal.id, edit: null },
                  {
                    onSuccess: () => {
                      setPending((current) => current.filter((item) => item.id !== proposal.id));
                      onConfirmed(missing, confirmed(proposal, null, missing));
                      setFilled(true);
                    },
                    onError: fail,
                  },
                )
              }
              onSaveEdit={(edit) =>
                confirm.mutate(
                  { id: proposal.id, edit },
                  {
                    onSuccess: () => {
                      setPending((current) => current.filter((item) => item.id !== proposal.id));
                      onConfirmed(missing, confirmed(proposal, edit, missing));
                      setFilled(true);
                    },
                    onError: fail,
                  },
                )
              }
              onDiscard={() =>
                discard.mutate(proposal.id, {
                  onSuccess: () =>
                    setPending((current) => current.filter((item) => item.id !== proposal.id)),
                  onError: fail,
                })
              }
            />
          ))}
        </ul>
      ) : null}
    </section>
  );
}
