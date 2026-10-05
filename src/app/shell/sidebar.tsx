import { Monitor, Moon, Sun } from "lucide-react";
import { useTranslation } from "react-i18next";
import { NavLink } from "react-router";
import { useTheme } from "@/app/theme";
import { currentLocale, setLocale } from "@/lib/i18n";
import { cn } from "@/lib/utils";
import { NAV_ITEMS, type RunState } from "./nav";

const navLinkBase =
  "flex items-center gap-3 rounded-xl px-3 py-2 text-sm font-medium transition-colors max-xl:justify-center max-xl:px-0";

const controlBase =
  "flex h-9 items-center gap-2 rounded-xl px-2 text-sm text-fg-muted hover:bg-surface-muted hover:text-fg";

export function Sidebar({ run }: { run: RunState }) {
  const { t } = useTranslation();

  return (
    <aside
      aria-label={t("shell:nav.aria")}
      className="flex h-full min-w-0 flex-col gap-2 border-r border-border bg-surface px-3 py-4"
    >
      <div className="flex h-10 items-center gap-2 px-2 max-xl:justify-center">
        <span aria-hidden className="size-6 shrink-0 rounded-full bg-primary" />
        <span className="font-display text-lg text-fg max-xl:hidden">Boba</span>
      </div>

      <nav className="min-h-0 flex-1 overflow-y-auto">
        <ul className="flex flex-col gap-1">
          {NAV_ITEMS.map((item) => (
            <li key={item.to}>
              <NavLink
                to={item.to}
                end={item.to === "/"}
                aria-label={t(item.labelKey)}
                title={t(item.labelKey)}
                className={({ isActive }) =>
                  cn(
                    navLinkBase,
                    isActive
                      ? "bg-primary-soft text-on-primary-soft"
                      : "text-fg-muted hover:bg-surface-muted hover:text-fg",
                  )
                }
              >
                <item.icon aria-hidden className="size-5 shrink-0" />
                <span className="truncate max-xl:hidden">{t(item.labelKey)}</span>
              </NavLink>
            </li>
          ))}
        </ul>
      </nav>

      <div className="flex flex-col gap-2 border-t border-border pt-3">
        <RunChip run={run} />
        <div className="flex items-center gap-2 max-xl:flex-col">
          <ThemeButton />
          <LanguageButton />
          <Tapi />
        </div>
      </div>
    </aside>
  );
}

function RunChip({ run }: { run: RunState }) {
  const { t } = useTranslation();
  const label = run.running ? t("shell:sidebar.run.running") : t("shell:sidebar.run.idle");
  return (
    <p className="flex items-center gap-2 px-2 text-xs text-fg-muted max-xl:justify-center">
      <span
        aria-hidden
        className={cn("size-2 shrink-0 rounded-full", run.running ? "bg-warning" : "bg-success")}
      />
      <span className="truncate max-xl:hidden">{label}</span>
    </p>
  );
}

function ThemeButton() {
  const { t } = useTranslation();
  const { theme, isDark, cycleTheme } = useTheme();
  const Icon = theme === "system" ? Monitor : isDark ? Moon : Sun;
  const mode = t(`shell:sidebar.theme.${theme}`);
  const label = t("shell:sidebar.theme.cycle", { mode });
  return (
    <button
      type="button"
      onClick={cycleTheme}
      title={label}
      aria-label={label}
      className={controlBase}
    >
      <Icon aria-hidden className="size-5 shrink-0" />
      <span className="max-xl:hidden">{mode}</span>
    </button>
  );
}

function LanguageButton() {
  const { t, i18n } = useTranslation();
  const locale = currentLocale();
  const label = t("shell:sidebar.language.toggle", { locale: locale.toUpperCase() });
  return (
    <button
      type="button"
      onClick={() => setLocale(locale === "en" ? "es" : "en")}
      title={label}
      aria-label={label}
      lang={i18n.language}
      className={cn(controlBase, "font-semibold uppercase max-xl:px-0")}
    >
      {locale}
    </button>
  );
}

function Tapi() {
  const { t } = useTranslation();
  return (
    <span
      role="img"
      aria-label={t("shell:sidebar.tapi")}
      className="grid size-9 shrink-0 place-items-center rounded-full bg-surface-muted max-xl:hidden"
    >
      <span aria-hidden className="size-3 rounded-full bg-accent" />
    </span>
  );
}
