import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { useEmailAccounts } from "@/features/connections/email/use-email";
import type { EmailApplyView, SendReceipt, SendRequest } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { useEmailReview, useEmailSend } from "./use-apply-bar";

/**
 * Email Apply in Review mode (S9.3, FR-82/FR-88, AC1).
 *
 * The user reads the recipient, edits the subject and body if they want, sees
 * the verified PDFs that will be attached and presses Send. The core re-checks
 * the package (AD-14), refuses a second send from the attempt evidence (AC2)
 * and writes the audit `intent`/`outcome` around the send (AD-12) — this screen
 * shows and confirms, it never decides.
 */
export default function EmailApplyPanel({ applicationId }: { applicationId: string }) {
  const { t } = useTranslation("apply_bar");
  const review = useEmailReview(applicationId);

  if (review.isPending) {
    return <p className="text-xs text-fg-muted">{t("email.loading")}</p>;
  }
  if (review.isError) {
    // A posting with no usable address is not an error to shout about: the
    // panel simply is not there.
    if (isAppError(review.error) && review.error.message_key === "errors.apply.no_email_channel") {
      return null;
    }
    return (
      <p role="alert" className="text-xs text-warning">
        {t("email.unavailable")}
      </p>
    );
  }
  return <EmailApplyForm applicationId={applicationId} view={review.data} />;
}

function EmailApplyForm({ applicationId, view }: { applicationId: string; view: EmailApplyView }) {
  const { t } = useTranslation("apply_bar");
  const { t: tError } = useTranslation();
  const accounts = useEmailAccounts();
  const send = useEmailSend(applicationId);
  const [accountId, setAccountId] = useState<string>("");
  const [subject, setSubject] = useState<string>(view.subject);
  const [body, setBody] = useState<string>(view.body_text);
  const [sent, setSent] = useState<SendReceipt | null>(null);

  const alreadySent = view.last_attempt?.state === "sent";
  const accountsList = accounts.data ?? [];
  const chosen = accountId || accountsList[0]?.id || "";
  const failure = send.error;

  const submit = (confirmResend: boolean) => {
    const request: SendRequest = {
      account_id: chosen,
      subject: subject === view.subject ? null : subject,
      body_text: body === view.body_text ? null : body,
      confirm_resend: confirmResend,
    };
    send.mutate(request, {
      onSuccess: (receipt) => setSent(receipt),
    });
  };

  if (sent) {
    return (
      <p className="text-sm text-fg" role="status" data-email-sent="">
        {t("email.sent", { id: sent.message_id })}
      </p>
    );
  }

  // AC2: the core asks for the user's word when an earlier send never answered.
  const needsConfirmation =
    isAppError(failure) && failure.message_key === "errors.apply.confirm_resend";
  const blocked = alreadySent || !view.ready || chosen.length === 0 || send.isPending;

  return (
    <div className="flex flex-col gap-2 rounded-2xl border border-border bg-surface p-4">
      <h3 className="font-display text-base text-fg">{t("email.title")}</h3>

      <p className="text-sm text-fg" data-email-recipient={view.recipient}>
        <span className="text-fg-muted">{t("email.to")}: </span>
        {view.recipient}
      </p>

      {accountsList.length === 0 ? (
        <p className="text-xs text-warning">{t("email.no_account")}</p>
      ) : (
        <label className="flex flex-col gap-1 text-xs text-fg-muted">
          {t("email.from")}
          <select
            className="rounded-lg border border-border bg-surface px-2 py-1 text-sm text-fg"
            value={chosen}
            onChange={(event) => setAccountId(event.target.value)}
          >
            {accountsList.map((account) => (
              <option key={account.id} value={account.id}>
                {account.address}
              </option>
            ))}
          </select>
        </label>
      )}

      <label className="flex flex-col gap-1 text-xs text-fg-muted">
        {t("email.subject")}
        <input
          className="rounded-lg border border-border bg-surface px-2 py-1 text-sm text-fg"
          value={subject}
          onChange={(event) => setSubject(event.target.value)}
        />
      </label>

      <label className="flex flex-col gap-1 text-xs text-fg-muted">
        {t("email.body")}
        <textarea
          className="min-h-32 rounded-lg border border-border bg-surface px-2 py-1 text-sm text-fg"
          value={body}
          onChange={(event) => setBody(event.target.value)}
        />
      </label>

      <div className="text-xs text-fg-muted">
        <span className="font-medium uppercase">{t("email.attachments")}</span>
        {view.attachments.length === 0 ? (
          <p>{t("email.no_attachments")}</p>
        ) : (
          <ul data-email-attachments="">
            {view.attachments.map((attachment) => (
              <li key={attachment.file_name}>
                {attachment.file_name} · {formatSize(attachment.bytes)}
              </li>
            ))}
          </ul>
        )}
      </div>

      {!view.ready ? (
        <p role="alert" className="text-xs text-warning">
          {t("email.not_ready")}
        </p>
      ) : null}

      {failure ? (
        <p role="alert" className="text-xs text-warning">
          {isAppError(failure) ? renderAppError(failure, tError) : t("failed")}
        </p>
      ) : null}

      <div className="flex flex-wrap items-center gap-2">
        <Button disabled={blocked} onClick={() => submit(needsConfirmation)}>
          {needsConfirmation ? t("email.send_anyway") : t("email.send")}
        </Button>
      </div>
    </div>
  );
}

/** Human size for the attachment list. */
function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
