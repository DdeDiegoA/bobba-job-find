import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { SessionBoardView } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import {
  useSessionOptIn,
  useSessionOptOut,
  useSessionResume,
  useSessionSignOut,
  useSessionSnapshot,
} from "./use-session";

/**
 * The per-Source Session Board opt-in (S11.1a, FR-30, AD-9). The order is the
 * acceptance criteria's: read a versioned warning that names the account-ban
 * risk and this platform's cookie protection, then confirm. On Linux without a
 * keyring a second, separate checkbox appears, and the core refuses the opt-in
 * without it (AC2).
 */

/** The cookie sentence per platform the core reported (AC2). */
const COOKIE_SENTENCE: Record<string, string> = {
  macos_keychain: "session_optin:warning.cookies.macos_keychain",
  windows_dpapi: "session_optin:warning.cookies.windows_dpapi",
  linux_keyring: "session_optin:warning.cookies.linux_keyring",
  linux_weak: "session_optin:warning.cookies.linux_weak",
};

/** The short store name the privacy page uses, per protection kind. */
export const PROTECTION_STORE: Record<string, string> = {
  macos_keychain: "session_optin:protection.macos_keychain",
  windows_dpapi: "session_optin:protection.windows_dpapi",
  linux_keyring: "session_optin:protection.linux_keyring",
  linux_weak: "session_optin:protection.linux_weak",
};

export interface SessionOptinProps {
  sourceId: string;
  sourceName: string;
}

/** The session card for one Session Board Source. */
export default function SessionOptin({ sourceId, sourceName }: SessionOptinProps) {
  const { t } = useTranslation("session_optin");
  const { t: tError } = useTranslation();
  const snapshot = useSessionSnapshot();
  const [warningOpen, setWarningOpen] = useState(false);
  const [read, setRead] = useState(false);
  const [weakAck, setWeakAck] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const optIn = useSessionOptIn();
  const optOut = useSessionOptOut();
  const signOut = useSessionSignOut();
  const resume = useSessionResume();
  const titleId = useId();
  const warningId = useId();

  const data = snapshot.data;
  const board = data?.boards.find((entry) => entry.source_id === sourceId);
  const needsWeakAck = data?.requires_extra_ack ?? false;
  const canConfirm = read && (!needsWeakAck || weakAck) && !optIn.isPending;
  const mutationError = optIn.error ?? optOut.error ?? signOut.error ?? resume.error ?? null;

  if (snapshot.isPending) {
    return <p className="text-sm text-fg-muted">{t("loading")}</p>;
  }
  if (snapshot.isError) {
    return (
      <p role="alert" className="text-sm text-danger">
        {isAppError(snapshot.error) ? renderAppError(snapshot.error, tError) : t("error.load")}
      </p>
    );
  }
  if (!board) return null;

  const closeWarning = () => {
    setWarningOpen(false);
    setRead(false);
    setWeakAck(false);
    optIn.reset();
  };

  const confirm = () => {
    if (!data) return;
    optIn.mutate(
      {
        sourceId,
        warningVersion: data.warning_version,
        weakCookieAck: needsWeakAck && weakAck,
      },
      { onSuccess: closeWarning },
    );
  };

  return (
    <section
      className="flex flex-col gap-3 rounded-2xl border border-border bg-surface p-4"
      aria-labelledby={titleId}
      data-source={sourceId}
    >
      <div className="flex items-start justify-between gap-3">
        <h3 id={titleId} className="font-display text-lg text-fg">
          {sourceName}
        </h3>
        <SessionState t={t} board={board} />
      </div>

      {board.opted_in && board.accepted_at ? (
        <p className="text-sm text-fg-muted">
          {t("state.accepted", {
            date: board.accepted_at,
            version: board.accepted_warning_version ?? data?.warning_version ?? "",
          })}
        </p>
      ) : null}

      {board.pause ? (
        <div className="flex flex-col gap-2 rounded-xl bg-surface-muted p-3 text-sm text-fg-muted">
          <p>
            {t("state.paused", {
              kind: t(`session_optin:challenge.${board.pause.kind}`),
              since: board.pause.since,
            })}
          </p>
          <div>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={resume.isPending}
              onClick={() =>
                resume.mutate(sourceId, {
                  onSuccess: () => {
                    setNotice(null);
                  },
                })
              }
            >
              {t("action.resume")}
            </Button>
          </div>
        </div>
      ) : null}

      {notice ? (
        <p role="status" className="text-sm text-fg-muted">
          {notice}
        </p>
      ) : null}

      {mutationError ? (
        <p role="alert" className="text-sm text-danger">
          {isAppError(mutationError) ? renderAppError(mutationError, tError) : t("error.save")}
        </p>
      ) : null}

      <div className="flex flex-wrap gap-2">
        {board.opted_in ? (
          <>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={signOut.isPending}
              onClick={() =>
                signOut.mutate(sourceId, {
                  onSuccess: () => {
                    setNotice(t("privacy.signed_out"));
                  },
                })
              }
            >
              {t("action.sign_out")}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={optOut.isPending}
              onClick={() => {
                setNotice(null);
                optOut.mutate(sourceId);
              }}
            >
              {t("action.disable")}
            </Button>
          </>
        ) : (
          <Button
            type="button"
            size="sm"
            disabled={warningOpen}
            onClick={() => setWarningOpen(true)}
          >
            {t("action.enable", { name: sourceName })}
          </Button>
        )}
      </div>

      {warningOpen && data ? (
        <section
          className="flex flex-col gap-3 rounded-xl border border-border bg-surface-muted p-4"
          aria-labelledby={warningId}
        >
          <h4 id={warningId} className="font-display text-base text-fg">
            {t("warning.title", { name: sourceName })}
          </h4>
          <p className="text-sm text-fg">{t("warning.ban", { name: sourceName })}</p>
          <p className="text-sm text-fg">
            {t(
              COOKIE_SENTENCE[data.cookie_protection] ?? "session_optin:warning.cookies.linux_weak",
            )}
          </p>
          <p className="text-sm text-fg-muted">{t("warning.retention")}</p>
          <p className="text-sm text-fg-muted">{t("warning.pacing")}</p>
          <p className="text-xs text-fg-muted">
            {t("warning.version", { version: data.warning_version })}
          </p>

          <label className="flex items-start gap-2 text-sm text-fg">
            <input
              type="checkbox"
              className="mt-1"
              checked={read}
              onChange={(event) => setRead(event.target.checked)}
            />
            <span>{t("warning.read")}</span>
          </label>

          {needsWeakAck ? (
            <label className="flex items-start gap-2 text-sm text-fg">
              <input
                type="checkbox"
                className="mt-1"
                checked={weakAck}
                onChange={(event) => setWeakAck(event.target.checked)}
              />
              <span>{t("warning.weak_ack")}</span>
            </label>
          ) : null}

          <div className="flex gap-2">
            <Button type="button" size="sm" disabled={!canConfirm} onClick={confirm}>
              {t("warning.confirm", { name: sourceName })}
            </Button>
            <Button type="button" size="sm" variant="ghost" onClick={closeWarning}>
              {t("action.cancel")}
            </Button>
          </div>
        </section>
      ) : null}
    </section>
  );
}

/** The on/off/paused chip. */
function SessionState({
  t,
  board,
}: {
  t: (key: string, options?: Record<string, unknown>) => string;
  board: SessionBoardView;
}) {
  const label = board.pause ? t("state.paused") : board.opted_in ? t("state.on") : t("state.off");
  const tone = board.pause
    ? "bg-surface-muted text-warning"
    : board.opted_in
      ? "bg-primary-soft text-on-primary-soft"
      : "bg-surface-muted text-fg-muted";
  return <span className={`rounded-xl px-2 py-1 text-xs ${tone}`}>{label}</span>;
}
