import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { Button, buttonVariants } from "@/components/ui/button";
import { ProposalReviewCard } from "@/features/proposal-review";
import { useCapability } from "@/lib/capabilities";
import { currentLocale } from "@/lib/i18n";
import {
  commands,
  type FactEdit,
  type GapAnswer,
  type ProposalView,
  type QuestionView,
} from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";
import { profileKey } from "./use-profile";

/** Where the user connects or changes the Agent CLI (FX7's Settings page). */
const AI_ENGINE_PATH = "/settings/ai-engine";

/** The CEFR levels the Gates understand (FR-12). */
const LEVELS = ["A1", "A2", "B1", "B2", "C1", "C2"] as const;

/** The authorizations a profile fact can state (the Eligibility Gate's set). */
const PERMITS = ["work_permit", "residency", "citizenship", "clearance"] as const;

/**
 * Re-run the profile gap interview from the Profile page (FX9, issue 5).
 *
 * The run is the same `profile_gaps` pass onboarding uses — the `profile_gaps`
 * capability gates it and the lockdown gate decides whether it may run at all.
 * It only proposes: the questions the agent wrote are shown verbatim, and every
 * fact it suggests waits as a pending proposal in the shared review card. One
 * answer or one confirmed proposal saves exactly that item (AC2); nothing else
 * reaches the profile.
 */
export function GapsEntry() {
  const { t } = useTranslation("onboarding_gaps");
  const { t: tError } = useTranslation();
  const { t: tGated } = useTranslation("gated");
  const access = useCapability("profile_gaps");
  const queryClient = useQueryClient();
  const language = currentLocale();

  const [questions, setQuestions] = useState<QuestionView[]>([]);
  const [proposals, setProposals] = useState<ProposalView[]>([]);
  const [asked, setAsked] = useState(false);
  const [errorText, setErrorText] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const fail = (error: unknown) => {
    setErrorText(isAppError(error) ? renderAppError(error, tError) : tError("errors.unknown"));
    setNotice(null);
  };

  const start = useMutation({
    mutationFn: () => commands.profileGapsStart(null, language).then(unwrap),
    onSuccess: (outcome) => {
      setQuestions(outcome.questions);
      // Append, never replace: a proposal from an earlier run stays reviewable.
      setProposals((current) => mergeProposals(current, outcome.proposals));
      setAsked(true);
      setErrorText(null);
      setNotice(null);
    },
    onError: fail,
  });

  const answer = useMutation({
    mutationFn: (input: { key: string; value: GapAnswer }) =>
      commands.profileGapsAnswer(input.key, input.value, language).then(unwrap),
    onSuccess: (_snapshot, input) => {
      // The gap is answered, so its question is no longer pending.
      setQuestions((current) => current.filter((item) => item.key !== input.key));
      setNotice(t("status.saved"));
      setErrorText(null);
      void queryClient.invalidateQueries({ queryKey: profileKey });
    },
    onError: fail,
  });

  const confirm = useMutation({
    mutationFn: (input: { id: string; edit: FactEdit | null }) =>
      commands.profileGapsConfirm(input.id, input.edit).then(unwrap),
    onSuccess: (snapshot, input) => {
      setProposals((current) => current.filter((item) => item.id !== input.id));
      queryClient.setQueryData(profileKey, snapshot);
      setNotice(t("status.saved"));
      setErrorText(null);
    },
    onError: fail,
  });

  const discard = useMutation({
    mutationFn: (id: string) => commands.profileGapsDiscard(id).then(unwrap),
    onSuccess: (_result, id) => {
      setProposals((current) => current.filter((item) => item.id !== id));
      setErrorText(null);
      void queryClient.invalidateQueries({ queryKey: profileKey });
    },
    onError: fail,
  });

  const noticeKey =
    access.reason === "cli_unsupported"
      ? { title: "notice.unsupportedTitle", body: "notice.unsupportedBody" }
      : { title: "notice.limitedTitle", body: "notice.limitedBody" };

  return (
    <section aria-labelledby="profile-gaps-title" className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <h2 id="profile-gaps-title" className="font-display text-lg text-fg">
          {t("review.title")}
        </h2>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={!access.available || start.isPending}
          onClick={() => {
            setErrorText(null);
            setNotice(null);
            start.mutate();
          }}
        >
          {start.isPending ? t("review.running") : t("review.action")}
        </Button>
      </div>
      <p className="text-sm text-fg-muted">{t("review.hint")}</p>

      {/* FX9 AC3: an unavailable action always says why and links to the engine. */}
      {!access.available && !access.isLoading ? (
        <div
          className="flex flex-col items-start gap-1 rounded-xl bg-surface-muted p-3 text-sm"
          role="status"
        >
          <p className="font-medium text-fg">{tGated(noticeKey.title)}</p>
          <p className="text-fg-muted">{tGated(noticeKey.body)}</p>
          <Link to={AI_ENGINE_PATH} className={buttonVariants({ variant: "outline", size: "sm" })}>
            {t("review.openSettings")}
          </Link>
        </div>
      ) : null}

      {errorText ? (
        <p className="text-sm text-danger" role="alert">
          {errorText}
        </p>
      ) : null}
      {start.isPending ? (
        <p className="text-sm text-fg-muted" role="status">
          {t("review.running")}
        </p>
      ) : null}
      {notice ? (
        <p role="status" className="text-sm text-success">
          {notice}
        </p>
      ) : null}

      {questions.length > 0 ? (
        <ul className="flex flex-col gap-3" aria-label={t("review.questions")}>
          {questions.map((question) => (
            <GapQuestion
              key={question.key}
              question={question}
              language={language}
              busy={answer.isPending}
              onSave={(value) => answer.mutate({ key: question.key, value })}
            />
          ))}
        </ul>
      ) : null}

      {proposals.length > 0 ? (
        <section aria-labelledby="profile-gaps-proposals" className="flex flex-col gap-3">
          <h3 id="profile-gaps-proposals" className="font-medium text-fg">
            {t("review.proposals")}
          </h3>
          <ul className="flex flex-col gap-3">
            {proposals.map((proposal) => (
              <ProposalReviewCard
                key={proposal.id}
                proposal={proposal}
                busy={confirm.isPending || discard.isPending}
                labels={{
                  kindLabel: t(`profile:sections.${proposal.kind}`),
                  textEn: t("drafts.textEn"),
                  textEs: t("drafts.textEs"),
                  confirm: t("drafts.confirm"),
                  edit: t("drafts.edit"),
                  saveEdit: t("drafts.save"),
                  cancel: t("drafts.cancel"),
                  discard: t("drafts.delete"),
                  editHint: t("drafts.editHint"),
                  untrusted: t("drafts.untrusted"),
                }}
                onConfirm={() => confirm.mutate({ id: proposal.id, edit: null })}
                onSaveEdit={(edit) => confirm.mutate({ id: proposal.id, edit })}
                onDiscard={() => discard.mutate(proposal.id)}
              />
            ))}
          </ul>
        </section>
      ) : null}

      {asked && questions.length === 0 && proposals.length === 0 ? (
        <p className="text-sm text-fg-muted">{t("review.empty")}</p>
      ) : null}
    </section>
  );
}

/** One question as the run wrote it, with the input its kind needs (AC2). */
function GapQuestion({
  question,
  language,
  busy,
  onSave,
}: {
  question: QuestionView;
  language: "en" | "es";
  busy: boolean;
  onSave: (answer: GapAnswer) => void;
}) {
  const { t } = useTranslation("onboarding_gaps");
  const [region, setRegion] = useState("");
  const [permit, setPermit] = useState<string>("work_permit");
  const [level, setLevel] = useState<string>("B2");
  const [text, setText] = useState("");

  const id = `profile-gap-${question.key}`;
  /** The agent's wording in the UI language, falling back to the step's copy. */
  const agentText = question[language === "es" ? "text_es" : "text_en"];
  const wording =
    agentText ?? (question.kind === "cefr" ? t("question.cefr") : t(`question.${question.kind}`));

  const submit = () => {
    if (question.kind === "eligibility") onSave({ kind: "eligibility", region, permit });
    else if (question.kind === "cefr") onSave({ kind: "cefr", level });
    else if (question.kind === "notice_period") onSave({ kind: "notice_period", text });
    else onSave({ kind: "writing_sample", text });
  };

  return (
    <li className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-4">
      <h3 className="font-medium text-fg">{wording}</h3>

      {question.kind === "eligibility" ? (
        <div className="flex flex-wrap items-end gap-3">
          <div className="flex flex-col gap-1">
            <label htmlFor={`${id}-region`} className="text-sm font-medium text-fg">
              {t("answer.region.label")}
            </label>
            <input
              id={`${id}-region`}
              className="w-32 rounded-xl border border-border bg-surface p-2 text-fg"
              placeholder={t("answer.region.placeholder")}
              value={region}
              onChange={(event) => setRegion(event.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1">
            <label htmlFor={`${id}-permit`} className="text-sm font-medium text-fg">
              {t("answer.permit.label")}
            </label>
            <select
              id={`${id}-permit`}
              className="rounded-xl border border-border bg-surface p-2 text-fg"
              value={permit}
              onChange={(event) => setPermit(event.target.value)}
            >
              {PERMITS.map((kind) => (
                <option key={kind} value={kind}>
                  {t(`answer.permit.${kind}`)}
                </option>
              ))}
            </select>
          </div>
        </div>
      ) : null}

      {question.kind === "cefr" ? (
        <div className="flex flex-col gap-1">
          <label htmlFor={`${id}-level`} className="text-sm font-medium text-fg">
            {t("answer.level.label")}
          </label>
          <select
            id={`${id}-level`}
            className="w-32 rounded-xl border border-border bg-surface p-2 text-fg"
            value={level}
            onChange={(event) => setLevel(event.target.value)}
          >
            {LEVELS.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </div>
      ) : null}

      {question.kind === "notice_period" ? (
        <div className="flex flex-col gap-1">
          <label htmlFor={`${id}-notice`} className="text-sm font-medium text-fg">
            {t("answer.notice.label")}
          </label>
          <input
            id={`${id}-notice`}
            className="w-48 rounded-xl border border-border bg-surface p-2 text-fg"
            placeholder={t("answer.notice.placeholder")}
            value={text}
            onChange={(event) => setText(event.target.value)}
          />
        </div>
      ) : null}

      {question.kind === "writing_sample" ? (
        <div className="flex flex-col gap-1">
          <label htmlFor={`${id}-sample`} className="text-sm font-medium text-fg">
            {t("answer.sample.label")}
          </label>
          <textarea
            id={`${id}-sample`}
            className="min-h-24 rounded-xl border border-border bg-surface p-3 text-fg"
            placeholder={t("answer.sample.placeholder")}
            value={text}
            onChange={(event) => setText(event.target.value)}
          />
        </div>
      ) : null}

      <div className="flex flex-wrap gap-2">
        <Button disabled={busy} onClick={submit}>
          {t("actions.save")}
        </Button>
      </div>
    </li>
  );
}

/** Keep the proposals in arrival order without duplicating an id. */
function mergeProposals(current: ProposalView[], next: ProposalView[]): ProposalView[] {
  const ids = new Set(current.map((item) => item.id));
  return [...current, ...next.filter((item) => !ids.has(item.id))];
}
