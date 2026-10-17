import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { AtsSessionView, Handoff } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { useAtsConfirm, useAtsOptIn, useAtsStart, useSetAtsOptIn } from "./use-ats-assist";

type T = (key: string, options?: Record<string, unknown>) => string;

/**
 * ATS Form assist (S9.4, FR-82/FR-88, AD-10).
 *
 * Boba opens the user's own Chrome on a dedicated profile and fills the form
 * while the user watches. It never presses submit, never solves a CAPTCHA and
 * never signs in: on any of those the core stops and this panel says which
 * field or wall handed control back. After the user submits, the core reads the
 * page and records the evidence — or marks the card Needs check.
 */
export default function AtsAssist({ applicationId }: { applicationId: string }) {
  const { t } = useTranslation("apply_bar");
  const { t: tError } = useTranslation();
  const optedIn = useAtsOptIn();
  const setOptIn = useSetAtsOptIn();
  const start = useAtsStart(applicationId);
  const confirm = useAtsConfirm(applicationId);
  const [session, setSession] = useState<AtsSessionView | null>(null);

  const failure = setOptIn.error ?? start.error ?? confirm.error;

  return (
    <div
      className="flex flex-col gap-2 rounded-xl border border-border bg-surface-muted p-3"
      data-ats-assist=""
    >
      <h3 className="text-xs font-medium uppercase text-fg-muted">{t("ats.title")}</h3>

      {optedIn.isPending ? null : optedIn.data === true ? (
        <div className="flex flex-col gap-2">
          <p className="text-xs text-fg-muted">{t("ats.opt_in_warning")}</p>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              disabled={start.isPending}
              onClick={() =>
                start.mutate(undefined, {
                  onSuccess: (view) => setSession(view),
                })
              }
            >
              {t("ats.start")}
            </Button>
            <Button
              variant="ghost"
              disabled={setOptIn.isPending}
              onClick={() => setOptIn.mutate(false)}
            >
              {t("ats.opt_out")}
            </Button>
          </div>
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          <p className="text-xs text-fg-muted">{t("ats.opt_in_warning")}</p>
          <div>
            <Button disabled={setOptIn.isPending} onClick={() => setOptIn.mutate(true)}>
              {t("ats.opt_in")}
            </Button>
          </div>
        </div>
      )}

      {session ? (
        <div className="flex flex-col gap-1 text-xs" data-ats-session="">
          <p className="text-fg">
            {t("ats.filled", {
              count: session.filled.length,
              documents: session.attached.length,
            })}
          </p>
          {session.handoff ? (
            <p className="text-warning" role="status" data-handoff={session.handoff.kind}>
              {handoffText(session.handoff, t)}
            </p>
          ) : null}
          <div>
            <Button
              variant="outline"
              disabled={confirm.isPending}
              onClick={() => confirm.mutate(session.attempt_id)}
            >
              {t("ats.confirm")}
            </Button>
          </div>
        </div>
      ) : null}

      {confirm.data ? (
        <p
          className={confirm.data.kind === "applied" ? "text-fg" : "text-warning"}
          role={confirm.data.kind === "applied" ? undefined : "alert"}
          data-ats-outcome={confirm.data.kind}
        >
          {confirm.data.kind === "applied"
            ? t("ats.applied", { text: confirm.data.evidence.confirmation_text })
            : t("ats.needs_check")}
        </p>
      ) : null}

      {failure ? (
        <p className="text-xs text-warning" role="alert">
          {isAppError(failure) ? renderAppError(failure, tError) : t("ats.failed")}
        </p>
      ) : null}
    </div>
  );
}

/** The handoff's copy, in the user's language (AC2). */
function handoffText(handoff: Handoff, t: T): string {
  switch (handoff.kind) {
    case "captcha":
      return t("ats.handoff_captcha");
    case "login_wall":
      return t("ats.handoff_login_wall");
    case "unknown_required":
      return t("ats.handoff_unknown", { label: handoff.label ?? "" });
    case "missing_document":
      return t("ats.handoff_missing_document", { label: handoff.label ?? "" });
  }
}
