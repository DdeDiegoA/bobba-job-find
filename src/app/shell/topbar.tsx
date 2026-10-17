import { Search } from "lucide-react";
import type { RefObject } from "react";
import { useTranslation } from "react-i18next";
import { useLocation } from "react-router";
import KillSwitch from "@/app/header/kill-switch";
import { Button } from "@/components/ui/button";
import { RunProgress } from "@/features/today/run-report/run-progress";
import { currentLocale } from "@/lib/i18n";
import { pageTitleKey, type RunState } from "./nav";

interface TopbarProps {
  run: RunState;
  searchRef: RefObject<HTMLButtonElement | null>;
  onSearch: () => void;
}

export function Topbar({ run, searchRef, onSearch }: TopbarProps) {
  const { t } = useTranslation();
  const { pathname } = useLocation();

  return (
    <header className="flex h-14 shrink-0 items-center gap-3 border-b border-border bg-surface px-4">
      <h1 className="min-w-0 flex-1 truncate font-display text-lg text-fg">
        {t(pageTitleKey(pathname))}
      </h1>

      {/* ponytail: the palette (S6.7) takes over this trigger; today it just
          receives focus so Cmd/Ctrl+K and `/` are already wired. */}
      <button
        ref={searchRef}
        type="button"
        data-slot="search"
        onClick={onSearch}
        aria-keyshortcuts="Meta+K Control+K"
        aria-label={t("shell:topbar.search")}
        className="flex h-9 items-center gap-2 rounded-xl border border-border bg-bg px-3 text-sm text-fg-muted hover:border-border-strong"
      >
        <Search aria-hidden className="size-4" />
        <span className="max-lg:hidden">{t("shell:topbar.searchHint")}</span>
      </button>

      {/* ponytail: no handler yet; the run lifecycle (S2.2) starts a run over IPC. */}
      <Button
        type="button"
        size="sm"
        disabled={run.running}
        title={run.running ? t("shell:topbar.runNow.busy") : undefined}
      >
        {run.running ? t("shell:topbar.runNow.running") : t("shell:topbar.runNow")}
      </Button>

      {run.autoEnabled ? (
        <div data-slot="kill-switch" className="shrink-0">
          <KillSwitch />
        </div>
      ) : null}

      <RunProgress />
      <RunStatus run={run} />
    </header>
  );
}

function RunStatus({ run }: { run: RunState }) {
  const { t } = useTranslation();
  if (!run.lastRunAt) {
    return (
      <span className="text-xs text-fg-subtle max-lg:hidden">
        {t("shell:topbar.runStatus.empty")}
      </span>
    );
  }
  const time = new Intl.DateTimeFormat(currentLocale(), {
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(run.lastRunAt));
  return (
    <span className="text-xs text-fg-muted max-lg:hidden">
      {t("shell:topbar.runStatus.last", { time, count: run.newCount })}
    </span>
  );
}
