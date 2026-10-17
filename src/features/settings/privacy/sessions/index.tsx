import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { PROTECTION_STORE } from "@/features/connections/session-optin";
import {
  useSessionResume,
  useSessionSignOut,
  useSessionSnapshot,
} from "@/features/connections/session-optin/use-session";
import { isAppError, renderAppError } from "@/lib/ipc/errors";

/**
 * Settings → Privacy → Sessions (S11.1a, FR-30 AC4): what Boba keeps for every
 * Session Board Source, for how long, and how it is protected. Cookies live
 * until "Sign out / clear session" or Reset everything; this page is where the
 * user reads that and acts on it.
 */
export default function SettingsPrivacySessions() {
  const { t } = useTranslation("session_optin");
  const { t: tError } = useTranslation();
  const snapshot = useSessionSnapshot();
  const signOut = useSessionSignOut();
  const resume = useSessionResume();
  const [notice, setNotice] = useState<string | null>(null);

  const data = snapshot.data;
  const failure = snapshot.error ?? signOut.error ?? resume.error ?? null;

  return (
    <section className="flex flex-col gap-4 p-6" aria-labelledby="settings-sessions-title">
      <h1 id="settings-sessions-title" className="font-display text-2xl text-fg">
        {t("privacy.title")}
      </h1>
      <p className="text-sm text-fg-muted">{t("privacy.intro")}</p>

      {snapshot.isPending ? <p className="text-sm text-fg-muted">{t("loading")}</p> : null}
      {failure ? (
        <p role="alert" className="text-sm text-danger">
          {isAppError(failure) ? renderAppError(failure, tError) : t("error.load")}
        </p>
      ) : null}

      {data ? (
        <>
          <p className="rounded-xl bg-surface-muted px-3 py-2 text-sm text-fg-muted">
            {t("privacy.cookie_protection", {
              store: t(
                PROTECTION_STORE[data.cookie_protection] ?? "session_optin:protection.linux_weak",
              ),
            })}
          </p>

          {notice ? (
            <p role="status" className="text-sm text-fg-muted">
              {notice}
            </p>
          ) : null}

          {data.boards.length === 0 ? (
            <p className="text-sm text-fg-muted">{t("privacy.none")}</p>
          ) : (
            <ul className="flex flex-col gap-3">
              {data.boards.map((board) => (
                <li
                  key={board.source_id}
                  className="flex flex-col gap-2 rounded-2xl border border-border bg-surface p-4"
                  data-source={board.source_id}
                >
                  <div className="flex items-start justify-between gap-3">
                    <h2 className="font-display text-lg text-fg">{board.name}</h2>
                    <span
                      className={
                        board.pause
                          ? "rounded-xl bg-surface-muted px-2 py-1 text-xs text-warning"
                          : board.opted_in
                            ? "rounded-xl bg-primary-soft px-2 py-1 text-xs text-on-primary-soft"
                            : "rounded-xl bg-surface-muted px-2 py-1 text-xs text-fg-muted"
                      }
                    >
                      {board.pause
                        ? t("state.paused", {
                            kind: t(`session_optin:challenge.${board.pause.kind}`),
                            since: board.pause.since,
                          })
                        : board.opted_in
                          ? t("privacy.opted_in", { date: board.accepted_at ?? "" })
                          : t("privacy.off")}
                    </span>
                  </div>

                  {/* AC4: the retention sentence, per Session Board Source. */}
                  <p className="text-sm text-fg-muted">{t("privacy.retention")}</p>

                  <div className="flex flex-wrap gap-2">
                    {board.pause ? (
                      <Button
                        type="button"
                        size="sm"
                        variant="outline"
                        disabled={resume.isPending}
                        onClick={() =>
                          resume.mutate(board.source_id, {
                            onSuccess: () => setNotice(null),
                          })
                        }
                      >
                        {t("privacy.resume")}
                      </Button>
                    ) : null}
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      disabled={signOut.isPending}
                      onClick={() =>
                        signOut.mutate(board.source_id, {
                          onSuccess: () => setNotice(t("privacy.signed_out")),
                        })
                      }
                    >
                      {t("privacy.sign_out")}
                    </Button>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </>
      ) : null}
    </section>
  );
}
