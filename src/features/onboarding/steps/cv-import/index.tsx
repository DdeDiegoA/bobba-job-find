import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { commands, type FactEdit, type ImportOutcome, type ProposalView } from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";
import type { StepProps } from "../../wizard/machine";

/** The file types the picker offers (FR-5); the core decides what it can read. */
const EXTENSIONS = ["pdf", "docx", "zip", "txt", "md", "csv"];

/**
 * Step — Master CV import (FR-5, AC1). The core extracts the text and runs one
 * `profile_import` pass; every proposed fact is shown here with the CV words it
 * was read from, and the user confirms, edits or deletes each one. Nothing is
 * saved to the Profile until then (AD-5).
 *
 * The step is optional: a user with no CV on hand adds facts by hand in the
 * Profile editor, and the gap interview (S3.6) fills what is missing.
 */
export default function CvImportStep({ answers, setAnswer }: StepProps) {
  const { t, i18n } = useTranslation();
  const { t: tError } = useTranslation();
  const queryClient = useQueryClient();

  const files = (answers.files as string[] | undefined) ?? [];
  const pastedText = (answers.pastedText as string | undefined) ?? "";
  // The last import's counts only: the wizard persists `answers` to
  // localStorage, so the proposal texts (which the query already owns) stay out
  // of it.
  const [outcome, setOutcome] = useState<ImportOutcome | null>(null);
  const [errorText, setErrorText] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const proposalsQuery = useQuery({
    queryKey: ["profile_import", "proposals"],
    queryFn: () => commands.profileImportProposals().then(unwrap),
  });
  const proposals = proposalsQuery.data ?? [];

  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: ["profile_import", "proposals"] });
    // The Profile screen (and its pending count) sees the new version.
    void queryClient.invalidateQueries({ queryKey: ["profile"] });
  };

  const fail = (error: unknown) => {
    setErrorText(isAppError(error) ? renderAppError(error, tError) : tError("errors.unknown"));
    // A run that died may still have stored what it proposed before it did.
    void queryClient.invalidateQueries({ queryKey: ["profile_import", "proposals"] });
  };

  const start = useMutation({
    mutationFn: () =>
      commands
        .profileImportStart(files, pastedText.trim() === "" ? null : pastedText, i18n.language)
        .then(unwrap),
    onSuccess: (result) => {
      setOutcome(result);
      setErrorText(null);
      setNotice(null);
      setAnswer("files", files);
      setAnswer("pastedText", pastedText);
      // The run's own answer is the waiting list; the stale read it replaces
      // would show fewer facts for a moment.
      queryClient.setQueryData(["profile_import", "proposals"], result.proposals);
      void queryClient.invalidateQueries({ queryKey: ["profile"] });
    },
    onError: fail,
  });

  const pickFiles = async () => {
    const picked = await open({
      multiple: true,
      filters: [{ name: t("onboarding_cv:pick.files"), extensions: EXTENSIONS }],
    }).catch(() => null);
    if (!picked) return;
    const paths = (Array.isArray(picked) ? picked : [picked]).filter(Boolean) as string[];
    const next = [...new Set([...files, ...paths])];
    setAnswer("files", next);
  };

  const confirm = useMutation({
    mutationFn: (input: { id: string; edit: FactEdit | null }) =>
      commands.profileImportConfirm(input.id, input.edit).then(unwrap),
    onSuccess: () => {
      setNotice(t("onboarding_cv:confirmed"));
      setErrorText(null);
      refresh();
    },
    onError: fail,
  });

  const confirmAll = useMutation({
    mutationFn: async (ids: string[]) => {
      for (const id of ids) {
        await commands.profileImportConfirm(id, null).then(unwrap);
      }
    },
    onSuccess: () => {
      setNotice(t("onboarding_cv:confirmed"));
      setErrorText(null);
      refresh();
    },
    onError: fail,
  });

  const discard = useMutation({
    mutationFn: (id: string) => commands.profileImportDiscard(id).then(unwrap),
    onSuccess: () => {
      setNotice(t("onboarding_cv:discarded"));
      setErrorText(null);
      refresh();
    },
    onError: fail,
  });

  const busy = confirm.isPending || confirmAll.isPending;
  const canImport = files.length > 0 || pastedText.trim() !== "";

  return (
    <div className="flex flex-col gap-6">
      <p className="text-fg-muted">{t("onboarding_cv:step.body")}</p>

      <div className="flex flex-col gap-2 rounded-xl border border-dashed border-border bg-surface-muted p-4">
        <div className="flex flex-wrap items-center gap-3">
          <Button variant="outline" onClick={() => void pickFiles()}>
            {t("onboarding_cv:pick.action")}
          </Button>
          <span className="text-sm text-fg-muted">{t("onboarding_cv:pick.files")}</span>
        </div>
        {files.length > 0 ? (
          <>
            <p className="text-sm text-fg">
              {t("onboarding_cv:pick.selected", { count: files.length })}
            </p>
            <ul className="flex flex-col gap-1">
              {files.map((path) => (
                <li key={path} className="break-all font-mono text-xs text-fg-subtle">
                  {path}
                </li>
              ))}
            </ul>
          </>
        ) : null}
        <p className="text-sm text-fg-muted">{t("onboarding_cv:step.privacy")}</p>
      </div>

      <div className="flex flex-col gap-2">
        <label htmlFor="onboarding-cv-paste" className="font-medium text-fg">
          {t("onboarding_cv:paste.title")}
        </label>
        <textarea
          id="onboarding-cv-paste"
          className="min-h-24 rounded-xl border border-border bg-surface p-3 text-fg"
          placeholder={t("onboarding_cv:paste.placeholder")}
          value={pastedText}
          onChange={(event) => setAnswer("pastedText", event.target.value)}
        />
      </div>

      <div className="flex flex-wrap items-center gap-3">
        <Button disabled={!canImport || start.isPending} onClick={() => start.mutate()}>
          {start.isPending ? t("onboarding_cv:import.running") : t("onboarding_cv:import.action")}
        </Button>
      </div>

      {errorText ? <p className="text-sm text-danger">{errorText}</p> : null}

      {outcome && !start.isPending ? (
        <p role="status" className="text-sm text-fg">
          {outcome.dropped > 0
            ? t("onboarding_cv:import.acceptedWithDrops", {
                accepted: outcome.accepted,
                dropped: outcome.dropped,
              })
            : outcome.accepted > 0
              ? t("onboarding_cv:import.accepted", { accepted: outcome.accepted })
              : t("onboarding_cv:import.empty")}
        </p>
      ) : null}
      {notice ? (
        <p role="status" className="text-sm text-success">
          {notice}
        </p>
      ) : null}

      <section className="flex flex-col gap-3" aria-labelledby="onboarding-cv-review">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h2 id="onboarding-cv-review" className="font-medium text-fg">
            {t("onboarding_cv:review.title")}
          </h2>
          {proposals.length > 1 ? (
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => confirmAll.mutate(proposals.map((item) => item.id))}
            >
              {t("onboarding_cv:review.confirmAll", { count: proposals.length })}
            </Button>
          ) : null}
        </div>
        <p className="text-sm text-fg-muted">{t("onboarding_cv:review.hint")}</p>
        {proposalsQuery.isError ? (
          <p className="text-sm text-danger">
            {isAppError(proposalsQuery.error)
              ? renderAppError(proposalsQuery.error, tError)
              : tError("errors.unknown")}
          </p>
        ) : null}
        {proposals.length === 0 ? (
          <p className="text-sm text-fg-muted">{t("onboarding_cv:review.empty")}</p>
        ) : (
          <>
            <p className="text-sm text-fg-muted">
              {t("onboarding_cv:review.count", { count: proposals.length })}
            </p>
            <ul className="flex flex-col gap-3">
              {proposals.map((proposal) => (
                <ProposalCard
                  key={proposal.id}
                  proposal={proposal}
                  busy={busy || discard.isPending}
                  onConfirm={() => confirm.mutate({ id: proposal.id, edit: null })}
                  onSave={(edit) => confirm.mutate({ id: proposal.id, edit })}
                  onDelete={() => discard.mutate(proposal.id)}
                />
              ))}
            </ul>
          </>
        )}
      </section>
    </div>
  );
}

/**
 * One proposed fact (AC1/AC3). Its text is shown **verbatim as plain text** —
 * whitespace preserved, never truncated, never rendered as markup (AD-7,
 * AD-15) — with the CV excerpt it was read from, and Confirm/Edit/Delete.
 */
function ProposalCard({
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

  const id = `cv-proposal-${proposal.id}`;

  if (editing) {
    return (
      <li className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-4">
        <h3 className="font-medium text-fg">{t(`profile:sections.${proposal.kind}`)}</h3>
        <label htmlFor={`${id}-en`} className="text-sm font-medium text-fg">
          {t("onboarding_cv:item.textEn")}
        </label>
        <textarea
          id={`${id}-en`}
          className="min-h-20 rounded-xl border border-border bg-surface p-3 text-fg"
          value={textEn}
          onChange={(event) => setTextEn(event.target.value)}
        />
        <label htmlFor={`${id}-es`} className="text-sm font-medium text-fg">
          {t("onboarding_cv:item.textEs")}
        </label>
        <textarea
          id={`${id}-es`}
          className="min-h-20 rounded-xl border border-border bg-surface p-3 text-fg"
          value={textEs}
          onChange={(event) => setTextEs(event.target.value)}
        />
        <p className="text-sm text-fg-muted">{t("onboarding_cv:item.editHint")}</p>
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
            {t("onboarding_cv:item.save")}
          </Button>
          <Button variant="ghost" onClick={() => setEditing(false)}>
            {t("onboarding_cv:item.cancel")}
          </Button>
        </div>
      </li>
    );
  }

  return (
    <li className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-4">
      <h3 className="font-medium text-fg">{t(`profile:sections.${proposal.kind}`)}</h3>
      {proposal.text_en ? (
        <div className="flex flex-col gap-1">
          <span className="text-xs uppercase tracking-wide text-fg-subtle">
            {t("onboarding_cv:item.textEn")}
          </span>
          <p className="whitespace-pre-wrap break-words text-fg">{proposal.text_en}</p>
        </div>
      ) : null}
      {proposal.text_es ? (
        <div className="flex flex-col gap-1">
          <span className="text-xs uppercase tracking-wide text-fg-subtle">
            {t("onboarding_cv:item.textEs")}
          </span>
          <p className="whitespace-pre-wrap break-words text-fg">{proposal.text_es}</p>
        </div>
      ) : null}
      {proposal.evidence ? (
        <p className="whitespace-pre-wrap break-words text-sm text-fg-muted">{proposal.evidence}</p>
      ) : null}
      {proposal.source_excerpt ? (
        <div className="flex flex-col gap-1 rounded-lg bg-surface-muted p-3">
          <span className="text-xs font-medium uppercase tracking-wide text-fg-subtle">
            {t("onboarding_cv:item.source")}
          </span>
          <blockquote className="whitespace-pre-wrap break-words text-sm text-fg-muted">
            {proposal.source_excerpt}
          </blockquote>
        </div>
      ) : null}
      <div className="flex flex-wrap gap-2">
        <Button disabled={busy} onClick={onConfirm}>
          {t("onboarding_cv:item.confirm")}
        </Button>
        <Button variant="outline" disabled={busy} onClick={() => setEditing(true)}>
          {t("onboarding_cv:item.edit")}
        </Button>
        <Button variant="ghost" disabled={busy} onClick={onDelete}>
          {t("onboarding_cv:item.delete")}
        </Button>
      </div>
    </li>
  );
}
