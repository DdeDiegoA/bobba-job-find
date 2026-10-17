import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { SessionApplyView, SessionHandoff, SessionOutcome } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { useSessionConfirm, useSessionFill, useSessionStart } from "./use-session-assist";

type T = (key: string, options?: Record<string, unknown>) => string;

/**
 * Session Board apply, Review mode (S11.2, FR-91, AD-10).
 *
 * Boba opens the user's own Chrome on **that Source's** session profile — the
 * one the user signed in to — and fills the step on screen. The user reads
 * every field and presses submit themselves; Boba never signs in, never
 * answers a challenge and never submits. On a board that moves in steps, the
 * user presses Next in the window and asks Boba to fill the next step. After
 * the user submits, the core reads the page and records the evidence — or
 * marks the card Needs check, exactly as the ATS channel does.
 *
 * The account-ban warning stays on screen the whole time: the user opted in to
 * this Source in Connections, and automating a prohibited board can cost the
 * account (FR-30, AD-11).
 */
export default function SessionAssist({
  applicationId,
  board,
}: {
  applicationId: string;
  board: string;
}) {
  const { t } = useTranslation("apply_bar");
  const { t: tError } = useTranslation();
  const start = useSessionStart(applicationId);
  const fill = useSessionFill(applicationId);
  const confirm = useSessionConfirm(applicationId);
  const [session, setSession] = useState<SessionApplyView | null>(null);
  const [outcome, setOutcome] = useState<SessionOutcome | null>(null);
  // S11.3 L1: a premature Confirm is not terminal — the attempt and the window
  // stay open, and the user submits and checks again.
  const [notSubmitted, setNotSubmitted] = useState(false);

  const failure = start.error ?? fill.error ?? confirm.error;

  const onStart = () => {
    setOutcome(null);
    setNotSubmitted(false);
    start.mutate(undefined, { onSuccess: (view) => setSession(view) });
  };
  const onFill = () => {
    if (!session) return;
    fill.mutate(session.attempt_id, { onSuccess: (view) => setSession(view) });
  };
  const onConfirm = () => {
    if (!session) return;
    confirm.mutate(session.attempt_id, {
      onSuccess: (result) => {
        if (result.kind === "not_submitted") {
          setNotSubmitted(true);
          return;
        }
        setNotSubmitted(false);
        setOutcome(result);
      },
    });
  };

  return (
    <div
      className="flex flex-col gap-2 rounded-xl border border-warning/40 bg-surface-muted p-3"
      data-session-assist={board}
    >
      <h3 className="text-xs font-medium uppercase text-fg-muted">
        {t("session.title", { board: boardLabel(board, t) })}
      </h3>

      {/* Always visible: the board's terms prohibit automated applying. */}
      <p role="note" className="text-xs text-warning" data-tos-warning="">
        {t("session.ban_warning")}
      </p>

      {outcome?.kind === "applied" ? (
        <p className="text-xs text-fg" role="status" data-session-outcome="applied">
          {t("session.applied", { text: outcome.evidence.confirmation_text })}
        </p>
      ) : outcome?.kind === "needs_check" ? (
        <p className="text-xs text-warning" role="status" data-session-outcome="needs_check">
          {t("session.needs_check")}
        </p>
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          <Button disabled={start.isPending} onClick={onStart}>
            {t("session.start")}
          </Button>
          {session ? (
            <>
              <Button variant="outline" disabled={fill.isPending} onClick={onFill}>
                {t("session.fill_step")}
              </Button>
              <Button variant="outline" disabled={confirm.isPending} onClick={onConfirm}>
                {t("session.confirm")}
              </Button>
            </>
          ) : null}
        </div>
      )}

      {session && !outcome ? (
        <div className="flex flex-col gap-1 text-xs" data-session-step="">
          <p className="text-fg">
            {t("session.filled", {
              count: session.filled.length,
              documents: session.attached.length,
            })}
          </p>
          {session.handoff ? <HandoffLine handoff={session.handoff} t={t} /> : null}
          {session.unanswered.length > 0 ? (
            <p className="text-fg-muted" data-unanswered={session.unanswered.length}>
              {t("session.unanswered", {
                labels: session.unanswered.map((field) => field.label).join(", "),
              })}
            </p>
          ) : null}
        </div>
      ) : null}

      {notSubmitted && !outcome ? (
        <p className="text-xs text-warning" role="status" data-session-outcome="not_submitted">
          {t("session_auto:session.not_submitted")}
        </p>
      ) : null}

      {failure ? (
        <p className="text-xs text-warning" role="alert">
          {isAppError(failure) ? renderAppError(failure, tError) : t("session.failed")}
        </p>
      ) : null}
    </div>
  );
}

/** The board's name as the UI shows it. */
function boardLabel(board: string, t: T): string {
  if (board === "linkedin") return t("session.board.linkedin");
  if (board === "indeed") return t("session.board.indeed");
  return board;
}

/** The line that says why Boba handed control back. */
function HandoffLine({ handoff, t }: { handoff: SessionHandoff; t: T }) {
  const label = handoff.label ?? "";
  // One literal per kind, never a composed key: the i18n gate must see each
  // leaf it is asked to keep.
  const line = (() => {
    switch (handoff.kind) {
      case "captcha":
        return t("session.handoff_captcha", { label });
      case "login_wall":
        return t("session.handoff_login_wall", { label });
      case "unusual_activity":
        return t("session.handoff_unusual_activity", { label });
      case "rate_limited":
        return t("session.handoff_rate_limited", { label });
      case "embedded_frame":
        return t("session.handoff_embedded_frame", { label });
      case "unknown_required":
        return t("session.handoff_unknown", { label });
      case "missing_document":
        return t("session.handoff_missing_document", { label });
    }
  })();
  return (
    <p className="text-warning" role="status" data-handoff={handoff.kind}>
      {line}
    </p>
  );
}
