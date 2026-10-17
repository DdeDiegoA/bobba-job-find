import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { ChannelView, KitAnswerView } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import EmailApplyPanel from "./email-apply";
import AnswerApprovePanel from "./approve-panel";
import AtsAssist from "./ats-assist";
import { useApplyView, useExportPdfs, useMarkApplied } from "./use-apply-bar";

/**
 * The Studio's apply bar (S9.2, FR-81/FR-82, EXPERIENCE §9.5).
 *
 * It says how the Offer can be applied to — Email Apply, ATS Form or Manual —
 * and, for the manual path, hands the user the kit: the verified PDFs to
 * attach and the answers they approved. *Mark applied* moves the card through
 * the Tracker's one writer; the core re-checks Ready + verified first (AD-14),
 * so the UI block is a courtesy, never the gate.
 */
export default function StudioApplyBar({ applicationId }: { applicationId: string }) {
  const { t } = useTranslation("apply_bar");
  const { t: tError } = useTranslation();
  const view = useApplyView(applicationId);
  const mark = useMarkApplied(applicationId);
  const exported = useExportPdfs(applicationId);
  const [copied, setCopied] = useState<number | null>(null);

  if (view.isPending) {
    return <p className="text-xs text-fg-muted">{t("loading")}</p>;
  }
  if (view.isError) {
    const message = isAppError(view.error)
      ? renderAppError(view.error, tError)
      : tError("errors.unknown");
    return (
      <p role="alert" className="text-xs text-warning">
        {message}
      </p>
    );
  }

  const bar = view.data;
  const manualUrl = bar.channels.find((channel) => channel.kind === "manual")?.url ?? "";
  const applied = bar.status === "applied";
  const failure = mark.error ?? exported.error;
  const kinds = bar.pending.map((kind) => kindLabel(kind, t));
  const hasAtsForm = bar.channels.some((channel) => channel.kind === "ats_form");

  const copy = async (answer: KitAnswerView, index: number) => {
    const clipboard = navigator.clipboard;
    // A webview without the clipboard API still shows the text to select; the
    // label only says "Copied" once a write actually resolved (reviewer-1 F3).
    if (!clipboard?.writeText) return;
    try {
      await clipboard.writeText(answer.answer);
      setCopied(index);
    } catch {
      // Denied by the webview: nothing was copied, so the label stays "Copy".
    }
  };

  return (
    <section
      className="flex flex-col gap-3 rounded-2xl border border-border bg-surface p-4"
      aria-labelledby="apply-bar-title"
      data-apply-ready={bar.ready}
      data-apply-status={bar.status}
    >
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h2 id="apply-bar-title" className="font-display text-lg text-fg">
          {t("title")}
        </h2>
        <div className="flex flex-wrap items-center gap-2">
          <Button disabled={manualUrl.length === 0} onClick={() => void openUrl(manualUrl)}>
            {t("apply")}
          </Button>
          <Button
            variant="outline"
            disabled={!bar.ready || applied || mark.isPending}
            onClick={() => mark.mutate()}
          >
            {applied ? t("applied") : t("mark_applied")}
          </Button>
        </div>
      </div>
      <p className="text-xs text-fg-muted">{t("mark_applied_hint")}</p>

      <div>
        <h3 className="text-xs font-medium uppercase text-fg-muted">{t("channels.title")}</h3>
        <ul className="mt-1 flex flex-col gap-1 text-sm" data-channels="">
          {bar.channels.map((channel) => (
            <li key={channel.kind} data-channel={channel.kind}>
              <span className="font-medium text-fg">{channelLabel(channel, t)}</span>
              <span className="text-fg-muted"> · {channelHint(channel, t)}</span>
            </li>
          ))}
        </ul>
      </div>

      {bar.channels.some((channel) => channel.kind === "email_apply") ? (
        <EmailApplyPanel applicationId={applicationId} />
      ) : null}

      {kinds.length > 0 ? (
        <p role="alert" className="text-xs text-warning" data-pending={bar.pending.join(",")}>
          {t("pending", { kinds: kinds.join(", ") })}
        </p>
      ) : null}

      {failure ? (
        <p className="text-xs text-warning" role="alert">
          {isAppError(failure) ? renderAppError(failure, tError) : t("failed")}
        </p>
      ) : null}

      {/* S9.4: the ATS Form channel can be filled in the user's own Chrome. */}
      {hasAtsForm ? <AtsAssist applicationId={applicationId} /> : null}

      <div className="flex flex-col gap-2">
        <h3 className="font-display text-base text-fg">{t("kit.title")}</h3>
        <h3 className="text-xs font-medium uppercase text-fg-muted">{t("kit.documents")}</h3>
        {bar.documents.length === 0 ? (
          <p className="text-xs text-fg-muted">{t("kit.no_documents")}</p>
        ) : (
          <ul className="flex flex-col gap-1 text-sm" data-kit-documents="">
            {bar.documents.map((document) => (
              <li key={document.document_id}>
                {kindLabel(document.kind, t)} · v{document.version}
              </li>
            ))}
          </ul>
        )}
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="outline"
            disabled={bar.documents.length === 0 || exported.isPending}
            onClick={() => exported.mutate()}
          >
            {t("kit.export")}
          </Button>
          {exported.isSuccess
            ? exported.data.map((file) => (
                <span key={file.path} className="flex items-center gap-2 text-xs text-fg-muted">
                  {t("kit.saved", { file: file.file_name })}
                  <button
                    type="button"
                    className="underline hover:text-fg"
                    onClick={() => void openPath(file.path)}
                  >
                    {t("kit.open")}
                  </button>
                </span>
              ))
            : null}
        </div>
      </div>

      <div className="flex flex-col gap-2">
        <h3 className="text-xs font-medium uppercase text-fg-muted">{t("kit.answers")}</h3>
        {bar.answers.length === 0 ? (
          <p className="text-xs text-fg-muted">{t("kit.no_answers")}</p>
        ) : (
          <ul className="flex flex-col gap-2 text-sm" data-kit-answers="">
            {bar.answers.map((answer, index) => (
              <li key={answer.question} className="flex flex-col gap-1">
                <span className="text-fg-muted">{answer.question}</span>
                <span className="flex items-start justify-between gap-2">
                  <span className="whitespace-pre-wrap text-fg">{answer.answer}</span>
                  <button
                    type="button"
                    className="shrink-0 text-xs underline hover:text-fg"
                    onClick={() => void copy(answer, index)}
                  >
                    {copied === index ? t("kit.copied") : t("kit.copy")}
                  </button>
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>

      {/* S7.8 surfaced by S9.4: approve the drafts the next form will reuse. */}
      <AnswerApprovePanel applicationId={applicationId} />
    </section>
  );
}

/** The channel's name. Literal keys, so the i18n gate sees them (AD-21). */
function channelLabel(
  channel: ChannelView,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  switch (channel.kind) {
    case "email_apply":
      return t("channels.email_apply");
    case "ats_form":
      return t("channels.ats_form");
    case "manual":
      return t("channels.manual");
  }
}

/** The document kind's name, in the user's language. */
function kindLabel(
  kind: string,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  return kind === "cv" ? t("kinds.cv") : t("kinds.cover_letter");
}

/** The hint under one channel, in the user's language. */
function channelHint(
  channel: ChannelView,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  switch (channel.kind) {
    case "email_apply":
      return t("channels.email_hint", { address: channel.address });
    case "ats_form":
      return t("channels.ats_hint", { provider: channel.provider });
    case "manual":
      return t("channels.manual_hint");
  }
}
