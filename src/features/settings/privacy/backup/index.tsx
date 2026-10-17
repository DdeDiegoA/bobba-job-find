import { open, save } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import {
  useBackupStatus,
  useCreateBackup,
  useResetEverything,
  useRestoreBackup,
} from "./use-backup";

/**
 * Settings → Privacy → Backup (S10.2): create a passphrase-encrypted backup,
 * restore one, and Reset everything. The forced recovery-backup requirement of
 * unsigned builds (ADR-009) is stated here; the onboarding step that refuses to
 * finish without one is S3.8's, and reads the same `backup_status`.
 */
export default function SettingsPrivacyBackup() {
  const { t } = useTranslation("backup");
  const { t: tError } = useTranslation();
  const status = useBackupStatus();

  const [destination, setDestination] = useState<string | null>(null);
  const [passphrase, setPassphrase] = useState("");
  const [confirmationPassphrase, setConfirmationPassphrase] = useState("");
  const [mismatch, setMismatch] = useState(false);

  const [restorePath, setRestorePath] = useState("");
  const [restorePassphrase, setRestorePassphrase] = useState("");
  const [restoreConfirmed, setRestoreConfirmed] = useState(false);

  const [typed, setTyped] = useState("");

  const create = useCreateBackup();
  const restore = useRestoreBackup();
  const reset = useResetEverything();

  const data = status.data;
  const token = data?.confirmation_token ?? "DELETE";
  const failure = status.error ?? create.error ?? restore.error ?? reset.error ?? null;

  const chooseDestination = async () => {
    const path = await save({
      title: t("create.choose"),
      defaultPath: "boba-backup.boba",
      filters: [{ name: "Boba backup", extensions: ["boba"] }],
    });
    if (typeof path === "string") setDestination(path);
  };

  const chooseBackup = async () => {
    const path = await open({
      title: t("restore.choose"),
      multiple: false,
      filters: [{ name: "Boba backup", extensions: ["boba"] }],
    });
    if (typeof path === "string") setRestorePath(path);
  };

  const onCreate = () => {
    if (passphrase !== confirmationPassphrase) {
      setMismatch(true);
      return;
    }
    setMismatch(false);
    if (!destination) return;
    create.mutate({ destination, passphrase });
  };

  const errorText = failure
    ? isAppError(failure)
      ? renderAppError(failure, tError)
      : t("error.action")
    : null;

  return (
    <section className="flex flex-col gap-6 p-6" aria-labelledby="settings-backup-title">
      <div className="flex flex-col gap-2">
        <h1 id="settings-backup-title" className="font-display text-2xl text-fg">
          {t("title")}
        </h1>
        <p className="text-sm text-fg-muted">{t("intro")}</p>
      </div>

      {status.isPending ? <p className="text-sm text-fg-muted">{t("loading")}</p> : null}
      {errorText ? (
        <p role="alert" className="text-sm text-danger">
          {errorText}
        </p>
      ) : null}

      {data ? (
        <div
          className="flex flex-col gap-1 rounded-2xl border border-border bg-surface p-4"
          data-testid="backup-status"
        >
          {data.recovery_present ? (
            <>
              <p role="status" className="text-sm text-fg">
                {t("status.present", { date: data.recovery_created_at ?? "" })}
              </p>
              {data.recovery_path ? (
                <p className="break-all text-xs text-fg-muted">
                  {t("status.path", { path: data.recovery_path })}
                </p>
              ) : null}
            </>
          ) : (
            <>
              <p className="text-sm text-warning">{t("status.missing")}</p>
              {data.recovery_required ? (
                <p className="text-sm text-fg-muted">{t("status.required")}</p>
              ) : null}
            </>
          )}
        </div>
      ) : null}

      {/* AC1: create one passphrase-encrypted file. */}
      <section
        className="flex flex-col gap-3 rounded-2xl border border-border p-4"
        aria-labelledby="backup-create-title"
      >
        <h2 id="backup-create-title" className="font-display text-lg text-fg">
          {t("create.title")}
        </h2>
        <p className="text-sm text-fg-muted">{t("create.intro")}</p>

        <div className="flex flex-wrap items-center gap-2">
          <Button type="button" variant="outline" size="sm" onClick={chooseDestination}>
            {t("create.choose")}
          </Button>
          <span className="break-all text-xs text-fg-muted">
            {destination ?? t("create.no_path")}
          </span>
        </div>

        <label className="flex flex-col gap-1 text-sm" htmlFor="backup-passphrase">
          {t("create.passphrase")}
          <input
            id="backup-passphrase"
            type="password"
            className="rounded-xl border border-border bg-surface px-3 py-2 text-fg"
            value={passphrase}
            onChange={(event) => setPassphrase(event.target.value)}
          />
        </label>
        <label className="flex flex-col gap-1 text-sm" htmlFor="backup-passphrase-confirm">
          {t("create.confirm")}
          <input
            id="backup-passphrase-confirm"
            type="password"
            className="rounded-xl border border-border bg-surface px-3 py-2 text-fg"
            value={confirmationPassphrase}
            onChange={(event) => setConfirmationPassphrase(event.target.value)}
          />
        </label>

        {mismatch ? (
          <p role="alert" className="text-sm text-danger">
            {t("create.mismatch")}
          </p>
        ) : null}

        <Button
          type="button"
          disabled={!destination || !passphrase || !confirmationPassphrase || create.isPending}
          onClick={onCreate}
        >
          {t("create.action")}
        </Button>

        {create.isSuccess ? (
          <p role="status" className="text-sm text-fg-muted">
            {t("create.done", { path: destination ?? "" })}
          </p>
        ) : null}
      </section>

      {/* AC3/AC4: validate, snapshot, replace. */}
      <section
        className="flex flex-col gap-3 rounded-2xl border border-border p-4"
        aria-labelledby="backup-restore-title"
      >
        <h2 id="backup-restore-title" className="font-display text-lg text-fg">
          {t("restore.title")}
        </h2>
        <p className="text-sm text-fg-muted">{t("restore.intro")}</p>

        <div className="flex flex-wrap items-center gap-2">
          <Button type="button" variant="outline" size="sm" onClick={chooseBackup}>
            {t("restore.choose")}
          </Button>
          <label
            className="flex min-w-0 flex-1 flex-col gap-1 text-sm"
            htmlFor="backup-restore-path"
          >
            {t("restore.path")}
            <input
              id="backup-restore-path"
              type="text"
              className="w-full rounded-xl border border-border bg-surface px-3 py-2 text-fg"
              value={restorePath}
              onChange={(event) => setRestorePath(event.target.value)}
            />
          </label>
        </div>

        <label className="flex flex-col gap-1 text-sm" htmlFor="backup-restore-passphrase">
          {t("restore.passphrase")}
          <input
            id="backup-restore-passphrase"
            type="password"
            className="rounded-xl border border-border bg-surface px-3 py-2 text-fg"
            value={restorePassphrase}
            onChange={(event) => setRestorePassphrase(event.target.value)}
          />
        </label>

        <label className="flex items-start gap-2 text-sm text-fg-muted">
          <input
            type="checkbox"
            checked={restoreConfirmed}
            onChange={(event) => setRestoreConfirmed(event.target.checked)}
          />
          {t("restore.confirm")}
        </label>

        <Button
          type="button"
          disabled={!restorePath || !restorePassphrase || !restoreConfirmed || restore.isPending}
          onClick={() => restore.mutate({ source: restorePath, passphrase: restorePassphrase })}
        >
          {t("restore.action")}
        </Button>

        {restore.isSuccess ? (
          <p role="status" className="text-sm text-fg-muted">
            {t("restore.done", {
              date: restore.data.created_at,
              snapshot: restore.data.snapshot,
            })}
          </p>
        ) : null}
      </section>

      {/* AC5: reset everything, with the uninstall notes. */}
      <section
        className="flex flex-col gap-3 rounded-2xl border border-danger/40 p-4"
        aria-labelledby="backup-reset-title"
      >
        <h2 id="backup-reset-title" className="font-display text-lg text-fg">
          {t("reset.title")}
        </h2>
        <p className="text-sm text-fg-muted">{t("reset.intro")}</p>

        <label className="flex flex-col gap-1 text-sm" htmlFor="backup-reset-confirmation">
          {t("reset.confirm_label", { token })}
          <input
            id="backup-reset-confirmation"
            type="text"
            autoComplete="off"
            className="rounded-xl border border-border bg-surface px-3 py-2 text-fg"
            value={typed}
            onChange={(event) => setTyped(event.target.value)}
          />
        </label>

        <Button
          type="button"
          variant="outline"
          className="border-danger text-danger"
          disabled={typed !== token || reset.isPending}
          onClick={() => reset.mutate(typed)}
        >
          {t("reset.action")}
        </Button>

        {reset.isSuccess ? (
          <div role="status" className="flex flex-col gap-1 text-sm text-fg-muted">
            <p className="text-fg">{t("reset.done")}</p>
            <p>{reset.data.db_removed ? t("reset.db_removed") : t("reset.db_kept")}</p>
            <p>{t("reset.keychain", { entries: reset.data.keychain_removed.length })}</p>
            {reset.data.remaining.length > 0 ? (
              <div>
                <p>{t("reset.remaining")}</p>
                <ul className="list-inside list-disc text-xs">
                  {reset.data.remaining.map((path) => (
                    <li key={path} className="break-all">
                      {path}
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
            <p className="break-all">{t("reset.uninstall", { path: reset.data.data_dir })}</p>
            <Button type="button" size="sm" variant="outline" onClick={() => void relaunch()}>
              {t("reset.restart")}
            </Button>
          </div>
        ) : null}
      </section>
    </section>
  );
}
