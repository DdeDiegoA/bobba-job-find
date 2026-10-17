import { CheckCheck, Inbox } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import type { StatusProposalView } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import {
  useApprove,
  useApproveMany,
  useClassify,
  useDisclosure,
  useDismiss,
  useProposals,
  useSaveDisclosure,
  useUndo,
} from "./use-needs-you";

/**
 * Today's needs-you queue (S8.4, FR-100..FR-103): what a recruiter reply says,
 * and the Status Proposal it produced — approve or dismiss, singly or in a
 * batch (AC2).
 *
 * The quoted line is the message's own words and an LLM-classified one is
 * marked: both are rendered as plain text, never as markup (AC6, ADR-010). The
 * disclosure card comes first the moment reply reading is enabled and the user
 * has not accepted it yet (AC3).
 */

/** The percentage a proposal's confidence is shown as. */
function percent(confidence: number | null): number {
  return Math.round((confidence ?? 0) * 100);
}

/** The disclosure: plain language, and the deterministic-only way out (AC3). */
function DisclosureCard() {
  const { t } = useTranslation("status_proposals");
  const save = useSaveDisclosure();
  return (
    <section
      aria-labelledby="disclosure-title"
      className="flex flex-col gap-3 rounded-2xl border border-border bg-surface p-4"
    >
      <h2 id="disclosure-title" className="font-display text-lg text-fg">
        {t("disclosure.title")}
      </h2>
      <p className="text-sm text-fg-muted">{t("disclosure.body")}</p>
      <p className="text-sm text-fg-muted">{t("disclosure.hint")}</p>
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          onClick={() => save.mutate({ mode: "llm", autoConfirm: false, acknowledge: true })}
          disabled={save.isPending}
        >
          {t("disclosure.allow")}
        </Button>
        <Button
          type="button"
          variant="outline"
          onClick={() =>
            save.mutate({ mode: "deterministic", autoConfirm: false, acknowledge: true })
          }
          disabled={save.isPending}
        >
          {t("disclosure.deterministic")}
        </Button>
      </div>
      {save.isError ? (
        <p role="alert" className="text-sm text-danger">
          {t("error.action")}
        </p>
      ) : null}
    </section>
  );
}

/** One proposal card. */
function Proposal({
  proposal,
  selected,
  onSelect,
  pick,
  onPick,
}: {
  proposal: StatusProposalView;
  selected: boolean;
  onSelect: (selected: boolean) => void;
  pick: string | null;
  onPick: (applicationId: string) => void;
}) {
  const { t } = useTranslation("status_proposals");
  const approve = useApprove();
  const dismiss = useDismiss();
  const where = proposal.application
    ? `${proposal.application.company} · ${proposal.application.title}`
    : t("proposal.unknown_application");

  return (
    <li
      className="flex flex-col gap-2 rounded-2xl border border-border bg-surface p-4"
      data-testid={`proposal-${proposal.id}`}
    >
      <div className="flex flex-wrap items-start justify-between gap-2">
        <label className="flex items-start gap-2">
          <input
            type="checkbox"
            className="mt-1"
            checked={selected}
            onChange={(event) => onSelect(event.target.checked)}
            aria-label={t("proposal.select", { subject: proposal.message_subject })}
          />
          <span className="flex flex-col">
            <span className="font-medium text-fg">
              {t(`status_proposals:class.${proposal.class}`)}
            </span>
            <span className="text-sm text-fg-muted">{where}</span>
          </span>
        </label>
        <span className="rounded-xl bg-surface-muted px-2 py-1 text-xs text-fg-muted">
          {t("proposal.status", { status: t(`status_proposals:status.${proposal.to_status}`) })} ·{" "}
          {t("proposal.confidence", { percent: percent(proposal.confidence) })}
        </span>
      </div>

      <p className="text-sm text-fg">{proposal.quoted_line}</p>
      {proposal.untrusted_origin ? (
        <p className="text-xs text-fg-subtle">{t("proposal.untrusted")}</p>
      ) : null}

      {proposal.needs_pick ? (
        <label className="flex flex-col gap-1 text-sm text-fg-muted">
          {t("proposal.pick")}
          <select
            className="rounded-xl border border-border bg-surface px-2 py-1 text-fg"
            value={pick ?? ""}
            onChange={(event) => onPick(event.target.value)}
          >
            <option value="" disabled>
              {t("proposal.pick_hint")}
            </option>
            {proposal.candidates.map((candidate) => (
              <option key={candidate.application_id} value={candidate.application_id}>
                {candidate.company} · {candidate.title}
              </option>
            ))}
          </select>
        </label>
      ) : null}

      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          size="sm"
          disabled={approve.isPending || (proposal.needs_pick && !pick)}
          onClick={() =>
            approve.mutate({ id: proposal.id, applicationId: proposal.needs_pick ? pick : null })
          }
        >
          {t("proposal.approve")}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          disabled={dismiss.isPending}
          onClick={() => dismiss.mutate(proposal.id)}
        >
          {t("proposal.dismiss")}
        </Button>
      </div>
      {approve.isError || dismiss.isError ? (
        <p role="alert" className="text-sm text-danger">
          {t("error.action")}
        </p>
      ) : null}
    </li>
  );
}

export default function TodayNeedsYou() {
  const { t } = useTranslation("status_proposals");
  const { t: tError } = useTranslation();
  const disclosure = useDisclosure();
  const proposals = useProposals();
  const classify = useClassify();
  const approveMany = useApproveMany();
  const undo = useUndo();
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [picks, setPicks] = useState<Record<string, string>>({});
  const [autoApplied, setAutoApplied] = useState<string[]>([]);

  const rows = useMemo(() => proposals.data ?? [], [proposals.data]);
  const chosen = rows.filter((row) => selected.has(row.id)).map((row) => row.id);

  const toggle = (id: string, on: boolean) => {
    setSelected((current) => {
      const next = new Set(current);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  };

  const check = () => {
    setAutoApplied([]);
    classify.mutate(undefined, {
      onSuccess: (outcome) => setAutoApplied(outcome.auto_applied_ids),
    });
  };

  return (
    <section className="flex flex-col gap-4 p-6" aria-labelledby="needs-you-title">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 id="needs-you-title" className="font-display text-2xl text-fg">
            {t("title")}
          </h1>
          <p className="text-sm text-fg-muted">{t("subtitle")}</p>
        </div>
        <Button type="button" onClick={check} disabled={classify.isPending}>
          {classify.isPending ? t("checking") : t("check")}
        </Button>
      </div>

      {classify.isError ? (
        <p role="alert" className="text-sm text-danger">
          {isAppError(classify.error) ? renderAppError(classify.error, tError) : t("error.action")}
        </p>
      ) : null}

      {autoApplied.length > 0 ? (
        <div className="flex flex-wrap items-center gap-2 rounded-2xl border border-border bg-surface-muted p-3 text-sm text-fg">
          <span>{t("auto_applied", { count: autoApplied.length })}</span>
          {autoApplied.map((id) => (
            <Button
              key={id}
              type="button"
              size="sm"
              variant="outline"
              onClick={() => undo.mutate(id)}
            >
              {t("undo")}
            </Button>
          ))}
        </div>
      ) : null}

      {classify.data && classify.data.uncertain > 0 ? (
        <p className="text-sm text-fg-muted">
          {t("uncertain", { count: classify.data.uncertain })}
        </p>
      ) : null}

      {disclosure?.data?.needs_disclosure ? <DisclosureCard /> : null}

      {proposals.isPending ? (
        <p className="text-sm text-fg-muted">{t("loading")}</p>
      ) : proposals.isError ? (
        <p role="alert" className="text-sm text-danger">
          {isAppError(proposals.error) ? renderAppError(proposals.error, tError) : t("error.load")}
        </p>
      ) : rows.length === 0 ? (
        <EmptyState icon={Inbox} title={t("empty.title")} description={t("empty.body")} />
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-2">
            <label className="flex items-center gap-2 text-sm text-fg-muted">
              <input
                type="checkbox"
                checked={chosen.length === rows.length}
                onChange={(event) =>
                  setSelected(event.target.checked ? new Set(rows.map((row) => row.id)) : new Set())
                }
              />
              {t("select_all")}
            </label>
            <Button
              type="button"
              size="sm"
              disabled={chosen.length === 0 || approveMany.isPending}
              onClick={() => approveMany.mutate(chosen)}
            >
              <CheckCheck aria-hidden className="size-4" /> {t("approve_selected")}
            </Button>
            {approveMany.isError ? (
              <span role="alert" className="text-sm text-danger">
                {t("error.action")}
              </span>
            ) : null}
            {approveMany.data && approveMany.data.needs_pick.length > 0 ? (
              <span className="text-sm text-fg-muted">
                {t("needs_pick", { count: approveMany.data.needs_pick.length })}
              </span>
            ) : null}
          </div>
          <ul className="flex flex-col gap-3">
            {rows.map((proposal) => (
              <Proposal
                key={proposal.id}
                proposal={proposal}
                selected={selected.has(proposal.id)}
                onSelect={(on) => toggle(proposal.id, on)}
                pick={picks[proposal.id] ?? null}
                onPick={(applicationId) =>
                  setPicks((current) => ({ ...current, [proposal.id]: applicationId }))
                }
              />
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
