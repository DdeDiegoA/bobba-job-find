import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import type { SessionAutoView } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import {
  useSessionAuto,
  useSessionAutoCap,
  useSessionAutoOptIn,
  useSessionAutoRevoke,
} from "./use-session-auto";

/**
 * Settings → Applying → Session (S11.3, FR-91).
 *
 * Auto on a Session Board is a **separate** opt-in from the reading one: the
 * user reads a versioned ToS/ban warning and types the Source name. The core
 * compares both, audits the acceptance, and enforces the limits (three a day by
 * default, ten at most; at least ten minutes apart; a seven-day pause after a
 * challenge; a 30-day re-confirmation). This screen only asks; it never decides.
 */
export default function SettingsApplyingSession() {
  const { t } = useTranslation("session_auto");
  const { t: tError } = useTranslation();
  const snapshot = useSessionAuto();
  const [error, setError] = useState<string | null>(null);

  const fail = (fallback: string) => (cause: unknown) =>
    setError(isAppError(cause) ? renderAppError(cause, tError) : fallback);

  return (
    <section className="flex flex-col gap-6 p-6" aria-labelledby="settings-session-title">
      <div>
        <h1 id="settings-session-title" className="font-display text-2xl text-fg">
          {t("title")}
        </h1>
        <p className="mt-2 text-sm text-fg-muted">{t("intro")}</p>
      </div>

      {snapshot.isPending ? <p className="text-sm text-fg-muted">{t("loading")}</p> : null}
      {snapshot.isError ? (
        <p role="alert" className="text-sm text-danger">
          {t("error.load")}
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      ) : null}

      {snapshot.data && snapshot.data.length === 0 ? (
        <EmptyState title={t("title")} description={t("empty")} />
      ) : null}

      <ul className="flex flex-col gap-3">
        {snapshot.data?.map((board) => (
          <li key={board.source_id}>
            <AutoCard board={board} onError={fail(t("error.save"))} />
          </li>
        ))}
      </ul>
    </section>
  );
}

const STATE_STYLE: Record<string, string> = {
  active: "bg-danger/15 text-danger",
  paused: "bg-warning/15 text-warning",
  expired: "bg-warning/15 text-warning",
  off: "bg-surface-muted text-fg-muted",
};

/** One literal per state, so the i18n gate sees every leaf it must keep. */
const STATE_LABEL: Record<string, string> = {
  active: "session_auto:state.active",
  paused: "session_auto:state.paused",
  expired: "session_auto:state.expired",
  off: "session_auto:state.off",
};

/** One Source's Auto card: the state, the warning gate and the cap. */
function AutoCard({
  board,
  onError,
}: {
  board: SessionAutoView;
  onError: (cause: unknown) => void;
}) {
  const { t } = useTranslation("session_auto");
  const optIn = useSessionAutoOptIn();
  const revoke = useSessionAutoRevoke();
  const cap = useSessionAutoCap();
  const [open, setOpen] = useState(false);
  const [read, setRead] = useState(false);
  const [typed, setTyped] = useState("");
  const [capDraft, setCapDraft] = useState(String(board.daily_cap));
  const titleId = useId();
  const formId = useId();

  const active = board.state === "active";
  const needsConfirm = board.state === "off" || board.state === "expired";
  // Revoking is always available once a record exists, paused or lapsed too.
  const canRevoke = board.state !== "off";
  const canConfirm = read && typed === board.name && !optIn.isPending;

  return (
    <section
      className="flex flex-col gap-3 rounded-2xl border border-border bg-surface p-4"
      aria-labelledby={titleId}
      data-source={board.source_id}
      data-auto-state={board.state}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <h2 id={titleId} className="font-display text-lg text-fg">
            {board.name}
          </h2>
          {board.state !== "off" ? (
            <span
              className="rounded-xl bg-danger/15 px-2 py-1 text-xs text-danger"
              data-tos-risk=""
            >
              {t("badge.tos_risk")}
            </span>
          ) : null}
        </div>
        <span className={`rounded-xl px-2 py-1 text-xs ${STATE_STYLE[board.state] ?? ""}`}>
          {t(STATE_LABEL[board.state] ?? "session_auto:state.off")}
        </span>
      </div>

      {board.state === "paused" && board.paused_until ? (
        <p className="text-sm text-warning">
          {t("state.paused_until", { until: board.paused_until })}
        </p>
      ) : null}
      {board.state === "expired" && board.expires_at ? (
        <p className="text-sm text-warning">{t("state.expired_at", { at: board.expires_at })}</p>
      ) : null}
      {active && board.accepted_at ? (
        <p className="text-sm text-fg-muted">
          {t("state.accepted", {
            date: board.accepted_at,
            until: board.expires_at ?? "",
          })}
        </p>
      ) : null}

      {!board.reading_opted_in ? (
        <p className="text-sm text-fg-muted">{t("needs_session")}</p>
      ) : null}

      {board.reading_opted_in ? (
        <div className="flex flex-col gap-2 rounded-xl bg-surface-muted p-3">
          <div className="flex flex-wrap items-center gap-2 text-sm">
            <label className="flex items-center gap-2 text-fg" htmlFor={`${formId}-cap`}>
              {t("cap.label")}
              <input
                id={`${formId}-cap`}
                className="w-16 rounded-lg border border-border bg-surface px-2 py-1 text-fg"
                type="number"
                min={1}
                max={board.max_daily_cap}
                value={capDraft}
                onChange={(event) => setCapDraft(event.target.value)}
              />
            </label>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={cap.isPending}
              onClick={() =>
                cap.mutate({ sourceId: board.source_id, cap: Number(capDraft) }, { onError })
              }
            >
              {t("cap.save")}
            </Button>
          </div>
          <p className="text-xs text-fg-muted">
            {t("cap.used", {
              used: board.used_today,
              cap: board.daily_cap,
              max: board.max_daily_cap,
            })}
          </p>
          <p className="text-xs text-fg-muted">{t("limits")}</p>
        </div>
      ) : null}

      <div className="flex flex-wrap gap-2">
        {canRevoke ? (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={revoke.isPending}
            onClick={() => revoke.mutate(board.source_id, { onError })}
          >
            {t("action.revoke")}
          </Button>
        ) : null}
        {needsConfirm && board.reading_opted_in ? (
          <Button
            type="button"
            size="sm"
            disabled={open}
            onClick={() => {
              setOpen(true);
              setRead(false);
              setTyped("");
            }}
          >
            {board.state === "expired"
              ? t("action.reconfirm", { name: board.name })
              : t("action.enable", { name: board.name })}
          </Button>
        ) : null}
      </div>

      {open ? (
        <section
          className="flex flex-col gap-3 rounded-xl border border-danger/40 bg-surface-muted p-4"
          aria-labelledby={`${formId}-warning`}
        >
          <h3 id={`${formId}-warning`} className="font-medium text-fg">
            {t("warning.title", { name: board.name })}
          </h3>
          <p className="text-sm text-fg">{t("warning.tos", { name: board.name })}</p>
          <p className="text-sm text-fg">{t("warning.ban", { name: board.name })}</p>
          <p className="text-sm text-fg-muted">{t("warning.limits", { name: board.name })}</p>
          <p className="text-xs text-fg-muted">
            {t("warning.version", { version: board.current_warning_version })}
          </p>

          <label className="flex items-start gap-2 text-sm text-fg">
            <input
              className="mt-1"
              type="checkbox"
              checked={read}
              onChange={(event) => setRead(event.target.checked)}
            />
            <span>{t("warning.read")}</span>
          </label>

          <label className="flex flex-col gap-1 text-sm text-fg" htmlFor={`${formId}-name`}>
            {t("warning.type", { name: board.name })}
            <input
              id={`${formId}-name`}
              className="rounded-lg border border-border bg-surface px-2 py-1 text-fg"
              value={typed}
              onChange={(event) => setTyped(event.target.value)}
              autoComplete="off"
              spellCheck={false}
            />
          </label>

          <div className="flex gap-2">
            <Button
              type="button"
              size="sm"
              disabled={!canConfirm}
              onClick={() =>
                optIn.mutate(
                  {
                    sourceId: board.source_id,
                    typedName: typed,
                    warningVersion: board.current_warning_version,
                  },
                  {
                    onSuccess: () => setOpen(false),
                    onError,
                  },
                )
              }
            >
              {t("warning.confirm", { name: board.name })}
            </Button>
            <Button type="button" size="sm" variant="ghost" onClick={() => setOpen(false)}>
              {t("action.cancel")}
            </Button>
          </div>
        </section>
      ) : null}
    </section>
  );
}
