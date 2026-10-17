import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Link, useSearchParams } from "react-router";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import type { StudioDocument, StudioKind } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { cn } from "@/lib/utils";
import StudioCoverage from "./coverage";
import StudioEditor, { type EditSlot } from "./editor";
import StudioTrace, { type TraceSentence } from "./trace";
import {
  useApplications,
  useStudioEdit,
  useStudioExport,
  useStudioGenerate,
  useStudioRegenerate,
  useStudioRender,
  useStudioView,
} from "./use-studio";

/**
 * Studio — tailoring per application (S7.7, FR-78/FR-79, EXPERIENCE §9.5).
 *
 * The screen reads one application's drafts through `?app=<id>` (the route is
 * S1.5's). Every action here is a request to the core: the editor never edits a
 * draft locally, so what the user reads is always a version the checks judged.
 * No-fabrication (FR-74) and ATS (FR-77) block *Ready*; *Send* belongs to
 * S9.2's apply bar and is only enabled on a verified, ready document.
 */
export default function StudioIndex() {
  const { t } = useTranslation("studio");
  const { t: tError } = useTranslation();
  const [params] = useSearchParams();
  const applicationId = params.get("app") ?? "";
  const applications = useApplications();
  const view = useStudioView(applicationId);
  const edit = useStudioEdit(applicationId);
  const render = useStudioRender(applicationId);
  const generate = useStudioGenerate(applicationId);
  const regenerate = useStudioRegenerate(applicationId);
  const exported = useStudioExport(applicationId);
  const [kind, setKind] = useState<StudioKind>("cv");

  if (applicationId.length === 0) {
    // The route names no application (S1.5's table has no parameter): offer the
    // ones the Tracker already holds, so the screen is always reachable.
    const cards = applications.data?.columns.flatMap((column) => column.cards) ?? [];
    return (
      <section className="flex flex-col gap-3 p-6" aria-labelledby="studio-pick-title">
        <h1 id="studio-pick-title" className="font-display text-2xl text-fg">
          {t("no_application.title")}
        </h1>
        <p className="text-sm text-fg-muted">{t("no_application.body")}</p>
        {cards.length === 0 ? null : (
          <ul className="flex max-w-md flex-col gap-2" data-applications="">
            {cards.map((card) => (
              <li key={card.id}>
                <Link
                  to={`/studio?app=${encodeURIComponent(card.id)}`}
                  className="flex items-center justify-between gap-2 rounded-xl border border-border px-3 py-2 text-sm text-fg hover:bg-surface-muted"
                >
                  <span>{card.title}</span>
                  <span className="text-xs text-fg-muted">{card.company}</span>
                </Link>
              </li>
            ))}
          </ul>
        )}
      </section>
    );
  }
  if (view.isPending) {
    return <p className="p-6 text-sm text-fg-muted">{t("loading")}</p>;
  }
  if (view.isError) {
    const message = isAppError(view.error)
      ? renderAppError(view.error, tError)
      : tError("errors.unknown");
    return (
      <p className="p-6 text-sm text-warning" role="alert">
        {message}
      </p>
    );
  }

  const studio = view.data;
  const document = kind === "cv" ? studio.cv : studio.cover;
  const busy = edit.isPending || render.isPending || generate.isPending || regenerate.isPending;
  const blocked = document !== null && !(document.ready && document.checks?.verified);
  // A refused action is the core's answer, shown in the user's language: an
  // edit that names a claim the draft lost, a run that could not start, an
  // export the disk refused (AD-3).
  const failure =
    edit.error ?? render.error ?? generate.error ?? regenerate.error ?? exported.error;

  const onEdit = (target: EditSlot, text: string) => {
    edit.mutate({ application_id: applicationId, kind, target, text });
  };
  const onRegenerate = (section: number | null) => {
    if (!document) return;
    regenerate.mutate({ kind, section, into: document.id });
  };

  return (
    <section className="flex min-h-0 flex-1 flex-col gap-4 p-6" aria-labelledby="studio-title">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <div>
          <h1 id="studio-title" className="font-display text-2xl text-fg">
            {t("title")}
          </h1>
          <p className="text-xs text-fg-muted">{t("hint")}</p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" disabled={busy} onClick={() => render.mutate()}>
            {t("actions.render")}
          </Button>
          <Button variant="outline" disabled={busy} onClick={() => generate.mutate()}>
            {t("actions.generate")}
          </Button>
          <Button
            disabled={busy || exported.isPending || document === null}
            onClick={() => exported.mutate(kind)}
          >
            {t("actions.export_docx")}
          </Button>
        </div>
      </div>

      {failure ? (
        <p className="text-xs text-warning" role="alert" data-failure="">
          {isAppError(failure) ? renderAppError(failure, tError) : tError("errors.unknown")}
        </p>
      ) : null}

      {exported.isSuccess ? (
        <p className="text-xs text-fg-muted" role="status" data-exported={exported.data.path}>
          {t("actions.exported", { file: exported.data.file_name })}
        </p>
      ) : null}

      <div className="flex min-h-0 flex-1 gap-4">
        <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3">
          <div role="tablist" aria-label={t("documents.aria")} className="flex gap-2">
            {(["cv", "cover_letter"] as const).map((tab) => (
              <button
                key={tab}
                type="button"
                role="tab"
                aria-selected={kind === tab}
                onClick={() => setKind(tab)}
                className={cn(
                  "rounded-xl px-3 py-1 text-sm",
                  kind === tab
                    ? "bg-primary-soft text-on-primary-soft"
                    : "text-fg-muted hover:text-fg",
                )}
              >
                {tab === "cv" ? t("documents.cv") : t("documents.cover_letter")}
              </button>
            ))}
          </div>

          {document ? (
            <StudioEditor
              document={document}
              onEdit={onEdit}
              onRegenerate={onRegenerate}
              busy={busy}
            />
          ) : (
            <EmptyState title={t("no_drafts.title")} description={t("no_drafts.body")} />
          )}
        </div>

        <aside className="flex w-80 shrink-0 flex-col gap-3 overflow-y-auto">
          <ChecksPanel document={document} blocked={blocked} />
          {document ? <FindingsPanel document={document} /> : null}
          {document ? (
            <StudioTrace
              sentences={trace(document)}
              onFix={(sentence) => {
                // "Fix" points the editor at the claim it names; the panel never
                // edits a draft itself.
                const target = trace(document).find((entry) => entry.text === sentence.text);
                if (target?.slot) {
                  edit.mutate({
                    application_id: applicationId,
                    kind,
                    target: target.slot,
                    text: sentence.text,
                  });
                }
              }}
            />
          ) : null}
          <StudioCoverage requirements={studio.requirements} />
        </aside>
      </div>
    </section>
  );
}

/** Every claim as the trace panel reads it, carrying the slot it sits in (S7.3). */
function trace(document: StudioDocument): (TraceSentence & { slot?: EditSlot })[] {
  const list: (TraceSentence & { slot?: EditSlot })[] = [];
  if (document.text.summary) {
    list.push({
      text: document.text.summary.text,
      facts: document.text.summary.facts,
      unknown: document.text.summary.unknown_facts,
      slot: { slot: "summary" },
    });
  }
  document.text.sections.forEach((section, sectionIndex) => {
    section.items.forEach((item, itemIndex) => {
      list.push({
        text: item.text,
        facts: item.facts,
        unknown: item.unknown_facts,
        slot: { slot: "cv_item", section: sectionIndex, item: itemIndex },
      });
    });
  });
  document.text.paragraphs.forEach((paragraph, index) => {
    list.push({
      text: paragraph.text,
      facts: paragraph.facts,
      unknown: paragraph.unknown_facts,
      slot: { slot: "cover_paragraph", index },
    });
  });
  return list;
}

/** The three checks and the gate they close (FR-74, FR-76, FR-77, AC1). */
function ChecksPanel({ document, blocked }: { document: StudioDocument | null; blocked: boolean }) {
  const { t } = useTranslation("studio");
  const checks = document?.checks ?? null;
  return (
    <section
      className="flex flex-col gap-2 rounded-2xl border border-border bg-surface p-4"
      aria-labelledby="studio-checks-title"
      data-blocked={blocked}
    >
      <h2 id="studio-checks-title" className="font-display text-lg text-fg">
        {t("checks.title")}
      </h2>
      {checks === null ? (
        <p className="text-xs text-fg-muted">{t("checks.none")}</p>
      ) : (
        <ul className="flex flex-col gap-1 text-sm">
          <li data-check="fabrication" data-passed={checks.fabrication_ready}>
            {checks.fabrication_ready ? t("checks.fabrication.ok") : t("checks.fabrication.failed")}
          </li>
          <li data-check="layout" data-passed={checks.layout.passes}>
            {t("checks.layout.line", {
              pages: checks.layout.pages,
              budget: checks.layout.page_budget,
            })}
          </li>
          <li data-check="text_layer" data-passed={checks.text_layer.passed}>
            {checks.text_layer.passed
              ? t("checks.text_layer.ok")
              : t("checks.text_layer.failed", {
                  count: checks.text_layer.missing_keywords.length,
                })}
          </li>
          <li data-check="verified" data-passed={checks.verified}>
            {checks.verified ? t("checks.verified") : t("checks.not_verified")}
          </li>
        </ul>
      )}
      <p className="text-xs" role="status">
        {blocked ? (
          <span className="text-warning">{t("checks.blocked")}</span>
        ) : (
          <span className="text-primary">{t("checks.ready")}</span>
        )}
      </p>
    </section>
  );
}

/** The reviewer's findings and the version they judged (S7.4). */
function FindingsPanel({ document }: { document: StudioDocument }) {
  const { t } = useTranslation("studio");
  const review = document.review;
  return (
    <section
      className="flex flex-col gap-2 rounded-2xl border border-border bg-surface p-4"
      aria-labelledby="studio-findings-title"
      data-findings={review?.findings.length ?? 0}
    >
      <h2 id="studio-findings-title" className="font-display text-lg text-fg">
        {t("findings.title")}
      </h2>
      {review === null ? (
        <p className="text-xs text-fg-muted">{t("findings.none")}</p>
      ) : (
        <>
          <p className="text-xs text-fg-subtle">
            {t("findings.on_version", { version: review.document_version })}
          </p>
          <ul className="flex flex-col gap-2">
            {review.findings.map((finding, index) => (
              <li
                // biome-ignore lint/suspicious/noArrayIndexKey: a finding's identity is its place in the reviewer's list, which the core preserves
                key={index}
                data-severity={finding.severity}
                className="flex flex-col gap-1 rounded-xl bg-surface-muted px-2 py-1"
              >
                <span className="text-xs text-fg-subtle">
                  {finding.severity === "high"
                    ? t("findings.severity.high")
                    : finding.severity === "medium"
                      ? t("findings.severity.medium")
                      : t("findings.severity.low")}
                </span>
                <span className="text-sm text-fg">{finding.message}</span>
                {finding.suggestion ? (
                  <span className="text-xs text-fg-muted">{finding.suggestion}</span>
                ) : null}
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
