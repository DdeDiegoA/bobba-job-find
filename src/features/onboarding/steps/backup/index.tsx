import { save } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { useBackupStatus, useCreateBackup } from "@/features/settings/privacy/backup/use-backup";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import type { StepProps } from "../../wizard/machine";

/**
 * Next unlocks once the core says a recovery backup exists (FR-111, ADR-009).
 * The step mirrors `backup_status` into its answers because the wizard's
 * validators only see answers; the core remains the one that decides.
 */
export function backupStepIsValid(answers: Record<string, unknown>): boolean {
  return answers.recoveryPresent === true;
}

/**
 * Step — recovery backup (FR-111, ADR-009). Every MVP build is unsigned, so no
 * first run happens until a passphrase-protected backup exists. The step says
 * in plain words why: the backup is how the user gets their data back if the OS
 * keychain entry is lost, and Boba cannot recover the passphrase (AC3).
 */
export default function BackupStep({ answers, setAnswer }: StepProps) {
  const { t } = useTranslation("onboarding_finish");
  const { t: tError } = useTranslation();
  const status = useBackupStatus();
  const create = useCreateBackup();

  // Kept in component state on purpose: the wizard's answers are persisted to
  // localStorage, and a passphrase must never be written there.
  const [destination, setDestination] = useState<string | null>(null);
  const [passphrase, setPassphrase] = useState("");
  const [again, setAgain] = useState("");
  const [mismatch, setMismatch] = useState(false);

  const data = status.data;
  const present = data?.recovery_present ?? false;

  // The wizard validator reads answers only, so the core's verdict is copied
  // there once per real change (never in a loop: equal values do not setState).
  useEffect(() => {
    if (!data) return;
    if (answers.recoveryPresent !== present) setAnswer("recoveryPresent", present);
  }, [data, answers.recoveryPresent, present, setAnswer]);

  const choose = async () => {
    const path = await save({
      title: t("backup.choose"),
      defaultPath: "boba-backup.boba",
      filters: [{ name: "Boba backup", extensions: ["boba"] }],
    });
    if (typeof path === "string") setDestination(path);
  };

  const onCreate = () => {
    if (passphrase !== again) {
      setMismatch(true);
      return;
    }
    setMismatch(false);
    if (!destination) return;
    create.mutate({ destination, passphrase });
  };

  const errorText = create.error
    ? isAppError(create.error)
      ? renderAppError(create.error, tError)
      : t("backup.error")
    : null;

  return (
    <div className="flex flex-col gap-6">
      <p className="text-fg-muted">{t("backup.intro")}</p>
      <p className="text-sm text-fg-muted">{t("backup.why")}</p>

      {status.isPending ? <p className="text-sm text-fg-muted">{t("backup.checking")}</p> : null}
      {status.isError ? (
        <div className="flex flex-wrap items-center gap-2">
          <p className="text-sm text-danger">{t("backup.loadError")}</p>
          <Button type="button" variant="outline" size="sm" onClick={() => void status.refetch()}>
            {t("backup.retry")}
          </Button>
        </div>
      ) : null}

      {data ? (
        <div
          className="flex flex-col gap-1 rounded-xl border border-border bg-surface-muted p-4"
          data-testid="onboarding-backup-status"
        >
          {data.recovery_present ? (
            <>
              <p role="status" className="text-sm text-fg">
                {t("backup.status.present", { date: data.recovery_created_at ?? "" })}
              </p>
              {data.recovery_path ? (
                <p className="break-all text-xs text-fg-muted">
                  {t("backup.status.path", { path: data.recovery_path })}
                </p>
              ) : null}
            </>
          ) : (
            <p className="text-sm text-warning">{t("backup.required")}</p>
          )}
        </div>
      ) : null}

      <section className="flex flex-col gap-3 rounded-xl border border-border p-4">
        <h3 className="font-medium text-fg">{t("backup.create.title")}</h3>

        <div className="flex flex-wrap items-center gap-2">
          <Button type="button" variant="outline" size="sm" onClick={() => void choose()}>
            {t("backup.choose")}
          </Button>
          <span className="break-all text-xs text-fg-muted">
            {destination ?? t("backup.noPath")}
          </span>
        </div>

        <label
          className="flex flex-col gap-1 text-sm text-fg"
          htmlFor="onboarding-backup-passphrase"
        >
          {t("backup.passphrase")}
          <input
            id="onboarding-backup-passphrase"
            type="password"
            className="rounded-xl border border-border bg-surface px-3 py-2 text-fg"
            value={passphrase}
            onChange={(event) => setPassphrase(event.target.value)}
          />
        </label>
        <label
          className="flex flex-col gap-1 text-sm text-fg"
          htmlFor="onboarding-backup-passphrase-again"
        >
          {t("backup.confirmPassphrase")}
          <input
            id="onboarding-backup-passphrase-again"
            type="password"
            className="rounded-xl border border-border bg-surface px-3 py-2 text-fg"
            value={again}
            onChange={(event) => setAgain(event.target.value)}
          />
        </label>

        {mismatch ? (
          <p role="alert" className="text-sm text-danger">
            {t("backup.mismatch")}
          </p>
        ) : null}

        <div>
          <Button
            type="button"
            disabled={!destination || !passphrase || !again || create.isPending}
            onClick={onCreate}
          >
            {create.isPending ? t("backup.creating") : t("backup.action")}
          </Button>
        </div>

        {errorText ? (
          <p role="alert" className="text-sm text-danger">
            {errorText}
          </p>
        ) : null}
      </section>
    </div>
  );
}
