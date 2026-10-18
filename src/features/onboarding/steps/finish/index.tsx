import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { toast } from "sonner";
import { rulesKey } from "@/features/settings/applying/rules/use-rules";
import { autostartKey } from "@/features/settings/notifications/use-autostart";
import i18n from "@/lib/i18n";
import { commands } from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";
import type { StepProps } from "../../wizard/machine";

/**
 * The default daily run time (FR-10): the scheduler's own default when no
 * schedule is stored (`boba_app::jobs::default_schedules`), so the copy and the
 * behaviour can never disagree.
 */
const RUN_TIME = "08:00";

/**
 * Finishing setup starts the first Search Run (FR-10, SM-1). The wizard calls
 * this from the last step's `onFinish`, so Next on the final step is the one
 * action that starts it. A run that cannot start (a locked store, no backup)
 * does not undo setup: the wizard still finishes and Today reports what ran —
 * but the failure is shown, never swallowed (FX14 AC3).
 */
export function finishSetup(): void {
  void commands
    .jobsRunNow()
    .then(unwrap)
    .catch((error: unknown) => {
      toast.error(
        isAppError(error)
          ? renderAppError(error, (key) => i18n.t(key))
          : i18n.t("onboarding_finish:finish.firstRunError"),
      );
    });
}

/**
 * Step — the last one (FR-10). It hands over the defaults the wizard settles:
 * Apply Mode Review (Auto cannot be enabled inside the wizard), a daily search
 * at 08:00 local time, and the launch-at-login state the OS actually has (S5.4,
 * FR-44) — never a hardcoded "Off". The first run starts when the user finishes,
 * with live progress in Today (AC4).
 */
export default function FinishStep(_props: StepProps) {
  const { t } = useTranslation("onboarding_finish");

  // The wizard never enables Auto, and a fresh install has it off (S9.6). The
  // snapshot keeps the line honest if an earlier setup left a rule enabled.
  const rules = useQuery({
    queryKey: rulesKey,
    queryFn: () => commands.applyRulesSnapshot().then(unwrap),
  });
  const autoOn = rules.data?.auto_enabled === true;

  // Launch at login is changed in Settings, not here: this reads the real state
  // and links there.
  const autostart = useQuery({
    queryKey: autostartKey,
    queryFn: () => commands.autostartStatus(),
  });

  return (
    <div className="flex flex-col gap-6">
      <p className="text-fg-muted">{t("finish.body")}</p>

      <dl className="flex flex-col gap-3">
        <div className="flex flex-col gap-1 rounded-xl border border-border bg-surface p-4">
          <dt className="text-sm font-medium text-fg">{t("finish.applyMode.title")}</dt>
          <dd className="text-sm text-fg">
            {autoOn ? t("finish.applyMode.autoOn") : t("finish.applyMode.review")}
          </dd>
          <dd className="text-sm text-fg-muted">
            {autoOn ? t("finish.applyMode.autoWarning") : t("finish.applyMode.explanation")}
          </dd>
        </div>
        <div className="flex flex-col gap-1 rounded-xl border border-border bg-surface p-4">
          <dt className="text-sm font-medium text-fg">{t("finish.schedule.title")}</dt>
          <dd className="text-sm text-fg">{t("finish.schedule.value", { time: RUN_TIME })}</dd>
          <dd className="text-sm text-fg-muted">{t("finish.schedule.hint")}</dd>
        </div>
        <div className="flex flex-col gap-1 rounded-xl border border-border bg-surface p-4">
          <dt className="text-sm font-medium text-fg">{t("finish.autostart.title")}</dt>
          <dd className="text-sm text-fg">
            {autostart.isPending
              ? t("finish.autostart.loading")
              : autostart.isError
                ? t("finish.autostart.error")
                : autostart.data.enabled
                  ? t("finish.autostart.on")
                  : t("finish.autostart.off")}
          </dd>
          {/* Only a state the core actually reported can be described (F2). */}
          {autostart.data ? (
            <dd className="text-sm text-fg-muted">
              {autostart.data.enabled
                ? t("finish.autostart.hintOn")
                : t("finish.autostart.hintOff")}
            </dd>
          ) : null}
          <dd className="text-sm">
            <Link
              to="/settings/notifications"
              className="text-primary underline-offset-2 hover:underline"
            >
              {t("finish.autostart.change")}
            </Link>
          </dd>
        </div>
      </dl>

      <p className="text-sm text-fg-muted">{t("finish.firstRun")}</p>
    </div>
  );
}
