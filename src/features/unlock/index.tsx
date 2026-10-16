import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { Button } from "@/components/ui/button";
import { commands, type UnlockStatus } from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";

/**
 * unlock/index.tsx — the Locked state screen (S1.2b, FR-109, ADR-009 §3).
 *
 * Reachable at `/unlock` (S1.5 routing). The store is locked when its key cannot
 * be read — a wiped keychain, an ACL change, or a machine with no Secret Service
 * — or when the passphrase key went idle. The two ways out are the keychain retry
 * (foreground, so the OS prompt can be answered) and the Argon2id passphrase.
 *
 * Restoring from a recovery backup is the third way out and belongs to S10.2,
 * which owns the backup file: this screen shows no affordance for it yet.
 */
const statusKey = ["unlock", "status"] as const;

/** The store state the screen renders (AC2). */
export function useUnlockStatus() {
  return useQuery({
    queryKey: statusKey,
    queryFn: () => commands.unlockStatus().then(unwrap),
  });
}

export default function Unlock() {
  const { t } = useTranslation("unlock");
  const { t: tCore } = useTranslation();
  const queryClient = useQueryClient();
  const status = useUnlockStatus();
  const [passphrase, setPassphrase] = useState("");
  const [problem, setProblem] = useState<string | null>(null);

  /** A core key that has no catalogue entry falls back, never to the raw key. */
  const coreText = (key: string) => {
    const text = tCore(key);
    return text === key ? tCore("errors.unknown") : text;
  };

  const failed = (error: unknown) =>
    setProblem(isAppError(error) ? renderAppError(error, tCore) : tCore("errors.unknown"));

  const settled = (next: UnlockStatus) => {
    queryClient.setQueryData(statusKey, next);
    setPassphrase("");
    setProblem(null);
  };

  const unlock = useMutation({
    mutationFn: (value: string) => commands.unlockWithPassphrase(value).then(unwrap),
    onSuccess: settled,
    onError: failed,
  });

  const retry = useMutation({
    mutationFn: () => commands.unlockRetryKeychain().then(unwrap),
    onSuccess: settled,
    onError: failed,
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!passphrase) return;
    setProblem(null);
    unlock.mutate(passphrase);
  };

  return (
    <section
      className="flex min-h-screen items-center justify-center bg-surface-muted p-6"
      aria-labelledby="unlock-title"
    >
      <div className="flex w-full max-w-md flex-col gap-3 rounded-2xl border border-border bg-surface p-6">
        <h1 id="unlock-title" className="font-display text-2xl text-fg">
          {status.data?.state === "ready" ? t("ready.title") : t("title")}
        </h1>

        {status.isPending ? <p className="text-sm text-fg-muted">{t("checking")}</p> : null}
        {status.isError ? (
          <p role="alert" className="text-sm text-danger">
            {isAppError(status.error)
              ? renderAppError(status.error, tCore)
              : tCore("errors.unknown")}
          </p>
        ) : null}

        {status.data?.state === "ready" ? (
          <>
            <p className="text-sm text-fg-muted">{t("ready.body")}</p>
            <Link
              to="/"
              className="inline-flex h-10 items-center justify-center rounded-xl bg-primary px-4 font-medium text-on-primary hover:bg-primary-hover"
            >
              {t("continue")}
            </Link>
          </>
        ) : null}

        {status.data?.state === "locked" ? (
          <>
            <p role="status" className="text-sm text-fg-muted">
              {coreText(status.data.message_key ?? "errors.unlock.locked")}
            </p>
            <p className="text-sm text-fg-muted">{t("body")}</p>

            <form className="flex flex-col gap-2" onSubmit={submit}>
              <label className="text-sm text-fg-muted" htmlFor="unlock-passphrase">
                {t("passphrase.label")}
              </label>
              <input
                id="unlock-passphrase"
                type="password"
                autoComplete="current-password"
                className="rounded-xl border border-border bg-surface px-3 py-2 text-fg"
                value={passphrase}
                onChange={(event) => setPassphrase(event.target.value)}
              />
              <Button type="submit" disabled={!passphrase || unlock.isPending}>
                {unlock.isPending ? t("unlocking") : t("unlock")}
              </Button>
            </form>

            {problem ? (
              <p role="alert" className="text-sm text-danger">
                {problem}
              </p>
            ) : null}

            <Button
              type="button"
              variant="outline"
              disabled={retry.isPending}
              onClick={() => {
                setProblem(null);
                retry.mutate();
              }}
            >
              {retry.isPending ? t("retrying") : t("retry")}
            </Button>
          </>
        ) : null}
      </div>
    </section>
  );
}
