import type { ComponentType } from "react";
import { useTranslation } from "react-i18next";
import { NavLink, useLocation } from "react-router";
import { cn } from "@/lib/utils";
import SettingsPreferences from "./preferences";

/**
 * Settings container (S3.7 owns it, backlog convention 7): a sub-page list plus
 * the page for the current `/settings/<id>` path. Each later settings slice
 * appends one entry — one line — to `SETTINGS_PAGES`.
 */
const SETTINGS_PAGES: readonly { id: string; labelKey: string; Component: ComponentType }[] = [
  { id: "preferences", labelKey: "preferences:title", Component: SettingsPreferences },
];

export default function SettingsIndex() {
  const { t } = useTranslation();
  const current = useLocation().pathname.replace(/^\/settings\/?/, "");
  const page = SETTINGS_PAGES.find((entry) => entry.id === current) ?? SETTINGS_PAGES[0];

  return (
    <section className="flex min-h-0 flex-1 gap-4" aria-labelledby="settings-title">
      <div className="flex w-56 shrink-0 flex-col gap-2 p-6">
        <h1 id="settings-title" className="font-display text-2xl text-fg">
          {t("shell:nav.settings")}
        </h1>
        <nav aria-label={t("shell:nav.settings")}>
          <ul className="flex flex-col gap-1">
            {SETTINGS_PAGES.map((entry) => (
              <li key={entry.id}>
                <NavLink
                  to={entry.id}
                  className={({ isActive }) =>
                    cn(
                      "block rounded-xl px-3 py-2 text-sm",
                      isActive
                        ? "bg-primary-soft text-on-primary-soft"
                        : "text-fg-muted hover:bg-surface-muted hover:text-fg",
                    )
                  }
                >
                  {t(entry.labelKey)}
                </NavLink>
              </li>
            ))}
          </ul>
        </nav>
      </div>

      <div className="min-w-0 flex-1 overflow-y-auto">
        <page.Component />
      </div>
    </section>
  );
}
