import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { rulesKey } from "@/features/settings/applying/rules/use-rules";
import { commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";
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
 * does not undo setup: the wizard still finishes and Today reports what ran.
 */
export function finishSetup(): void {
  void commands.jobsRunNow().catch(() => undefined);
}

/**
 * Step — the last one (FR-10). It hands over the defaults the wizard settles:
 * Apply Mode Review (Auto cannot be enabled inside the wizard), a daily search
 * at 08:00 local time and launch at login off. The first run starts when the
 * user finishes, with live progress in Today (AC4).
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
          <dd className="text-sm text-fg">{t("finish.autostart.off")}</dd>
          <dd className="text-sm text-fg-muted">{t("finish.autostart.hint")}</dd>
        </div>
      </dl>

      <p className="text-sm text-fg-muted">{t("finish.firstRun")}</p>
    </div>
  );
}
