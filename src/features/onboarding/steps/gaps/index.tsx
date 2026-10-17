import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  commands,
  type FactEdit,
  type FactView,
  type Gap,
  type GapAnswer,
  type ProposalView,
  type QuestionView,
} from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";
import type { StepProps } from "../../wizard/machine";

/** The CEFR levels the gates understand (FR-12). */
const LEVELS = ["A1", "A2", "B1", "B2", "C1", "C2"] as const;

/** The authorizations a profile fact can state (the Eligibility Gate's set). */
const PERMITS = ["work_permit", "residency", "citizenship", "clearance"] as const;

/**
 * Step — profile gap interview (FR-6, AC1). Boba asks only what the Profile
 * cannot answer: work eligibility, the CEFR level of each declared language,
 * the notice period and an optional writing sample. Every question can be
 * skipped, and a skipped question lowers the Confidence of the Gate that
 * needed it (AC1). The answer the user types is their own profile fact.
 *
 * The questions themselves are written by the `profile_gaps` agent run; the
 * core decides which are real and the step falls back to its own copy when no
 * engine is available (S3.3 limited mode).
 */
/** The key the answer and skip commands use, as the core spelled it. */
const gapKey = (gap: Gap): string =>
  gap.subject === null ? gap.kind : `${gap.kind}:${gap.subject}`;

export default function GapsStep({ setAnswer }: StepProps) {
  const { t, i18n } = useTranslation();
  const { t: tError } = useTranslation();
  const queryClient = useQueryClient();
  const language = i18n.language.startsWith("es") ? "es" : "en";

  const [written, setWritten] = useState<QuestionView[]>([]);
  const [drafts, setDrafts] = useState<ProposalView[]>([]);
  const [errorText, setErrorText] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const gapsQuery = useQuery({
    queryKey: ["profile_gaps", "questions"],
    queryFn: () => commands.profileGapsQuestions().then(unwrap),
  });
  const factsQuery = useQuery({
    queryKey: ["profile", "snapshot"],
    queryFn: () => commands.profileSnapshot().then(unwrap),
  });

  const gaps = gapsQuery.data ?? [];
  const missingVariants = (factsQuery.data?.facts ?? []).filter(
    (fact) => fact.missing_variant !== null,
  );

  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: ["profile_gaps"] });
    void queryClient.invalidateQueries({ queryKey: ["profile"] });
  };

  const fail = (error: unknown) => {
    setErrorText(isAppError(error) ? renderAppError(error, tError) : tError("errors.unknown"));
    // A run that died may still have stored the proposals it made before it did.
    refresh();
  };

  const ask = useMutation({
    mutationFn: () => commands.profileGapsStart(null, language).then(unwrap),
    onSuccess: (outcome) => {
      setWritten(outcome.questions);
      setDrafts((current) => mergeDrafts(current, outcome.proposals));
      setErrorText(null);
      setNotice(null);
      setAnswer("asked", true);
    },
    onError: fail,
  });

  const answer = useMutation({
    mutationFn: (input: { key: string; answer: GapAnswer }) =>
      commands.profileGapsAnswer(input.key, input.answer, language).then(unwrap),
    onSuccess: () => {
      setNotice(t("onboarding_gaps:status.saved"));
      setErrorText(null);
      refresh();
    },
    onError: fail,
  });

  const skip = useMutation({
    mutationFn: (key: string) => commands.profileGapsSkip(key).then(unwrap),
    onSuccess: () => {
      setNotice(t("onboarding_gaps:status.skipped"));
      setErrorText(null);
      refresh();
    },
    onError: fail,
  });

  const draft = useMutation({
    mutationFn: (input: { item_id: string; lang: string }) =>
      commands.profileGapsTranslate(input.item_id, input.lang, language).then(unwrap),
    onSuccess: (outcome) => {
      setDrafts((current) => mergeDrafts(current, outcome.proposals));
      setErrorText(null);
    },
    onError: fail,
  });

  const confirm = useMutation({
    mutationFn: (input: { id: string; edit: FactEdit | null }) =>
      commands.profileGapsConfirm(input.id, input.edit).then(unwrap),
    onSuccess: (_snapshot, input) => {
      setDrafts((current) => current.filter((item) => item.id !== input.id));
      setNotice(t("onboarding_gaps:status.saved"));
      setErrorText(null);
      refresh();
    },
    onError: fail,
  });

  const discard = useMutation({
    mutationFn: (id: string) => commands.profileGapsDiscard(id).then(unwrap),
    onSuccess: (_result, id) => {
      setDrafts((current) => current.filter((item) => item.id !== id));
      setErrorText(null);
      refresh();
    },
    onError: fail,
  });

  const busy = answer.isPending || skip.isPending;

  return (
    <div className="flex flex-col gap-6">
      <p className="text-fg-muted">{t("onboarding_gaps:step.body")}</p>

      <div className="flex flex-wrap items-center gap-3">
        <Button disabled={ask.isPending} onClick={() => ask.mutate()}>
          {ask.isPending ? t("onboarding_gaps:ask.running") : t("onboarding_gaps:ask.action")}
        </Button>
        <span className="text-sm text-fg-muted">{t("onboarding_gaps:ask.hint")}</span>
      </div>

      {errorText ? <p className="text-sm text-danger">{errorText}</p> : null}
      {notice ? (
        <p role="status" className="text-sm text-success">
          {notice}
        </p>
      ) : null}

      {gapsQuery.isError ? (
        <p className="text-sm text-danger">
          {isAppError(gapsQuery.error)
            ? renderAppError(gapsQuery.error, tError)
            : tError("errors.unknown")}
        </p>
      ) : null}

      {gaps.length === 0 && !gapsQuery.isLoading ? (
        <p className="text-sm text-fg-muted">{t("onboarding_gaps:languages.empty")}</p>
      ) : (
        <ul className="flex flex-col gap-4">
          {gaps.map((gap) => (
            <QuestionCard
              key={gapKey(gap)}
              gap={gap}
              written={written.find((question) => question.key === gapKey(gap))}
              language={language}
              busy={busy}
              onAnswer={(given) => answer.mutate({ key: gapKey(gap), answer: given })}
              onSkip={() => skip.mutate(gapKey(gap))}
            />
          ))}
        </ul>
      )}

      <section className="flex flex-col gap-3" aria-labelledby="onboarding-gaps-languages">
        <h2 id="onboarding-gaps-languages" className="font-medium text-fg">
          {t("onboarding_gaps:languages.title")}
        </h2>
        <p className="text-sm text-fg-muted">{t("onboarding_gaps:languages.body")}</p>
        {missingVariants.length === 0 ? (
          <p className="text-sm text-fg-muted">{t("onboarding_gaps:languages.empty")}</p>
        ) : (
          <ul className="flex flex-col gap-2">
            {missingVariants.map((fact) => (
              <LanguageRow
                key={fact.item_id}
                fact={fact}
                busy={draft.isPending}
                onDraft={(lang) => draft.mutate({ item_id: fact.item_id, lang })}
              />
            ))}
          </ul>
        )}
      </section>

      <section className="flex flex-col gap-3" aria-labelledby="onboarding-gaps-drafts">
        <h2 id="onboarding-gaps-drafts" className="font-medium text-fg">
          {t("onboarding_gaps:drafts.title")}
        </h2>
        {drafts.length === 0 ? (
          <p className="text-sm text-fg-muted">{t("onboarding_gaps:drafts.empty")}</p>
        ) : (
          <ul className="flex flex-col gap-3">
            {drafts.map((proposal) => (
              <DraftCard
                key={proposal.id}
                proposal={proposal}
                busy={confirm.isPending || discard.isPending}
                onConfirm={() => confirm.mutate({ id: proposal.id, edit: null })}
                onSave={(edit) => confirm.mutate({ id: proposal.id, edit })}
                onDelete={() => discard.mutate(proposal.id)}
              />
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

/** One question: the agent's wording when there is one, the step's own copy
 * otherwise, and the input its kind needs (AC1). */
function QuestionCard({
  gap,
  written,
  language,
  busy,
  onAnswer,
  onSkip,
}: {
  gap: Gap;
  written: QuestionView | undefined;
  language: "en" | "es";
  busy: boolean;
  onAnswer: (answer: GapAnswer) => void;
  onSkip: () => void;
}) {
  const { t } = useTranslation();
  const [region, setRegion] = useState("");
  const [permit, setPermit] = useState<string>("work_permit");
  const [level, setLevel] = useState<string>("B2");
  const [text, setText] = useState("");

  const id = `gap-${gapKey(gap)}`;
  /** The agent's question in the UI language, falling back to the step's copy. */
  const agentText = written?.[language === "es" ? "text_es" : "text_en"] ?? null;
  const question =
    agentText ??
    (gap.kind === "cefr"
      ? t("onboarding_gaps:question.cefr")
      : t(`onboarding_gaps:question.${gap.kind}`));

  const submit = () => {
    if (gap.kind === "eligibility") onAnswer({ kind: "eligibility", region, permit });
    else if (gap.kind === "cefr") onAnswer({ kind: "cefr", level });
    else if (gap.kind === "notice_period") onAnswer({ kind: "notice_period", text });
    else onAnswer({ kind: "writing_sample", text });
  };

  return (
    <li className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-4">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h3 className="font-medium text-fg">{question}</h3>
        {/* FR-6: the writing sample is the one question that can be skipped
            without costing a Gate anything. */}
        {gap.kind === "writing_sample" ? (
          <span className="text-xs uppercase tracking-wide text-fg-subtle">
            {t("onboarding_gaps:question.optional")}
          </span>
        ) : null}
      </div>

      {gap.kind === "eligibility" ? (
        <div className="flex flex-wrap items-end gap-3">
          <div className="flex flex-col gap-1">
            <label htmlFor={`${id}-region`} className="text-sm font-medium text-fg">
              {t("onboarding_gaps:answer.region.label")}
            </label>
            <input
              id={`${id}-region`}
              className="w-32 rounded-xl border border-border bg-surface p-2 text-fg"
              placeholder={t("onboarding_gaps:answer.region.placeholder")}
              value={region}
              onChange={(event) => setRegion(event.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1">
            <label htmlFor={`${id}-permit`} className="text-sm font-medium text-fg">
              {t("onboarding_gaps:answer.permit.label")}
            </label>
            <select
              id={`${id}-permit`}
              className="rounded-xl border border-border bg-surface p-2 text-fg"
              value={permit}
              onChange={(event) => setPermit(event.target.value)}
            >
              {PERMITS.map((kind) => (
                <option key={kind} value={kind}>
                  {t(`onboarding_gaps:answer.permit.${kind}`)}
                </option>
              ))}
            </select>
          </div>
        </div>
      ) : null}

      {gap.kind === "cefr" ? (
        <div className="flex flex-col gap-1">
          <label htmlFor={`${id}-level`} className="text-sm font-medium text-fg">
            {t("onboarding_gaps:answer.level.label")}
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

      {gap.kind === "notice_period" ? (
        <div className="flex flex-col gap-1">
          <label htmlFor={`${id}-notice`} className="text-sm font-medium text-fg">
            {t("onboarding_gaps:answer.notice.label")}
          </label>
          <input
            id={`${id}-notice`}
            className="w-48 rounded-xl border border-border bg-surface p-2 text-fg"
            placeholder={t("onboarding_gaps:answer.notice.placeholder")}
            value={text}
            onChange={(event) => setText(event.target.value)}
          />
        </div>
      ) : null}

      {gap.kind === "writing_sample" ? (
        <div className="flex flex-col gap-1">
          <label htmlFor={`${id}-sample`} className="text-sm font-medium text-fg">
            {t("onboarding_gaps:answer.sample.label")}
          </label>
          <textarea
            id={`${id}-sample`}
            className="min-h-24 rounded-xl border border-border bg-surface p-3 text-fg"
            placeholder={t("onboarding_gaps:answer.sample.placeholder")}
            value={text}
            onChange={(event) => setText(event.target.value)}
          />
        </div>
      ) : null}

      <div className="flex flex-wrap gap-2">
        <Button disabled={busy} onClick={submit}>
          {t("onboarding_gaps:actions.save")}
        </Button>
        <Button variant="ghost" disabled={busy} onClick={onSkip}>
          {t("onboarding_gaps:actions.skip")}
        </Button>
      </div>
    </li>
  );
}

/** A fact with one variant: draft the other, one button per missing language. */
function LanguageRow({
  fact,
  busy,
  onDraft,
}: {
  fact: FactView;
  busy: boolean;
  onDraft: (lang: string) => void;
}) {
  const { t } = useTranslation();
  const missing = fact.missing_variant ?? "en";
  const text = fact.missing_variant === "en" ? fact.text_es : fact.text_en;
  return (
    <li className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-border bg-surface p-3">
      <span className="whitespace-pre-wrap break-words text-fg">{text ?? fact.kind}</span>
      <Button variant="outline" disabled={busy} onClick={() => onDraft(missing)}>
        {busy
          ? t("onboarding_gaps:languages.drafting")
          : t("onboarding_gaps:languages.draft", { lang: missing.toUpperCase() })}
      </Button>
    </li>
  );
}

/**
 * One agent-drafted proposal (AC3). Its text is shown **verbatim as plain
 * text** — whitespace preserved, never truncated, never rendered as markup
 * (AD-7, AD-15) — and nothing is saved until the user confirms or edits it.
 */
function DraftCard({
  proposal,
  busy,
  onConfirm,
  onSave,
  onDelete,
}: {
  proposal: ProposalView;
  busy: boolean;
  onConfirm: () => void;
  onSave: (edit: FactEdit) => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [textEn, setTextEn] = useState(proposal.text_en ?? "");
  const [textEs, setTextEs] = useState(proposal.text_es ?? "");

  const id = `gap-draft-${proposal.id}`;

  if (editing) {
    return (
      <li className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-4">
        <h3 className="font-medium text-fg">{t(`profile:sections.${proposal.kind}`)}</h3>
        <label htmlFor={`${id}-en`} className="text-sm font-medium text-fg">
          {t("onboarding_gaps:drafts.textEn")}
        </label>
        <textarea
          id={`${id}-en`}
          className="min-h-20 rounded-xl border border-border bg-surface p-3 text-fg"
          value={textEn}
          onChange={(event) => setTextEn(event.target.value)}
        />
        <label htmlFor={`${id}-es`} className="text-sm font-medium text-fg">
          {t("onboarding_gaps:drafts.textEs")}
        </label>
        <textarea
          id={`${id}-es`}
          className="min-h-20 rounded-xl border border-border bg-surface p-3 text-fg"
          value={textEs}
          onChange={(event) => setTextEs(event.target.value)}
        />
        <p className="text-sm text-fg-muted">{t("onboarding_gaps:drafts.editHint")}</p>
        <div className="flex flex-wrap gap-2">
          <Button
            disabled={busy}
            onClick={() =>
              onSave({
                text_en: textEn.trim() === "" ? null : textEn,
                text_es: textEs.trim() === "" ? null : textEs,
                evidence: proposal.evidence,
                detail_json: proposal.detail_json,
              })
            }
          >
            {t("onboarding_gaps:drafts.save")}
          </Button>
          <Button variant="ghost" onClick={() => setEditing(false)}>
            {t("onboarding_gaps:drafts.cancel")}
          </Button>
        </div>
      </li>
    );
  }

  return (
    <li className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-4">
      <h3 className="font-medium text-fg">{t(`profile:sections.${proposal.kind}`)}</h3>
      {proposal.untrusted_origin ? (
        <p className="text-sm text-warning">{t("onboarding_gaps:drafts.untrusted")}</p>
      ) : null}
      {proposal.text_en ? (
        <div className="flex flex-col gap-1">
          <span className="text-xs uppercase tracking-wide text-fg-subtle">
            {t("onboarding_gaps:drafts.textEn")}
          </span>
          <p className="whitespace-pre-wrap break-words text-fg">{proposal.text_en}</p>
        </div>
      ) : null}
      {proposal.text_es ? (
        <div className="flex flex-col gap-1">
          <span className="text-xs uppercase tracking-wide text-fg-subtle">
            {t("onboarding_gaps:drafts.textEs")}
          </span>
          <p className="whitespace-pre-wrap break-words text-fg">{proposal.text_es}</p>
        </div>
      ) : null}
      <div className="flex flex-wrap gap-2">
        <Button disabled={busy} onClick={onConfirm}>
          {t("onboarding_gaps:drafts.confirm")}
        </Button>
        <Button variant="outline" disabled={busy} onClick={() => setEditing(true)}>
          {t("onboarding_gaps:drafts.edit")}
        </Button>
        <Button variant="ghost" disabled={busy} onClick={onDelete}>
          {t("onboarding_gaps:drafts.delete")}
        </Button>
      </div>
    </li>
  );
}

/** Keep the drafts in arrival order without duplicating an id. */
function mergeDrafts(current: ProposalView[], next: ProposalView[]): ProposalView[] {
  const ids = new Set(current.map((item) => item.id));
  return [...current, ...next.filter((item) => !ids.has(item.id))];
}
