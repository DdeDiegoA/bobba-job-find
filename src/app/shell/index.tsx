import { useCallback, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Outlet, useNavigate } from "react-router";
import CommandPalette from "@/app/palette";
import { useTheme } from "@/app/theme";
import { EmptyState } from "@/components/ui/empty-state";
import { NAV_ITEMS } from "./nav";
import { Sidebar } from "./sidebar";
import { Topbar } from "./topbar";
import { useRunNow } from "./use-run-now";

function isEditable(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el || typeof el.tagName !== "string") return false;
  return (
    el.tagName === "INPUT" ||
    el.tagName === "TEXTAREA" ||
    el.tagName === "SELECT" ||
    el.isContentEditable
  );
}

/**
 * Shell-level global shortcuts (DESIGN.md §10.4). List-scoped keys (J/K, C, A,
 * S, X, Enter) belong to their feature slices; the Kill Switch override is S9.8.
 */
function useShellShortcuts({
  focusSearch,
  toggleTheme,
}: {
  focusSearch: () => void;
  toggleTheme: () => void;
}) {
  const navigate = useNavigate();
  const gotoPending = useRef(false);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const mod = event.metaKey || event.ctrlKey;
      const key = event.key.toLowerCase();

      if (mod && key === "k" && !event.shiftKey) {
        event.preventDefault();
        focusSearch();
        return;
      }
      if (mod && event.shiftKey && key === "l") {
        event.preventDefault();
        toggleTheme();
        return;
      }
      if (mod && key === ",") {
        event.preventDefault();
        navigate("/settings");
        return;
      }
      if (mod && /^[1-7]$/.test(event.key)) {
        event.preventDefault();
        navigate(NAV_ITEMS[Number(event.key) - 1].to);
        return;
      }

      if (mod || event.altKey || isEditable(event.target)) return;

      if (event.key === "/") {
        event.preventDefault();
        focusSearch();
        return;
      }
      if (key === "g") {
        gotoPending.current = true;
        return;
      }
      if (gotoPending.current) {
        gotoPending.current = false;
        const item = NAV_ITEMS.find((nav) => nav.goto === key);
        if (item) {
          event.preventDefault();
          navigate(item.to);
        }
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [navigate, focusSearch, toggleTheme]);
}

/** Shell root (owned by S1.5): sidebar + topbar + routed content. */
export function Shell() {
  const { t } = useTranslation();
  const { toggleTheme } = useTheme();
  const searchRef = useRef<HTMLButtonElement>(null);
  const { run, message, pending, runNow } = useRunNow();
  const focusSearch = useCallback(() => searchRef.current?.focus(), []);
  useShellShortcuts({ focusSearch, toggleTheme });

  return (
    <>
      <a href="#main" className="skip-link">
        {t("shell:shell.skipToContent")}
      </a>
      <div className="shell grid h-full grid-cols-[15rem_minmax(0,1fr)] max-xl:grid-cols-[4rem_minmax(0,1fr)]">
        <Sidebar run={run} />
        <div className="flex h-full min-w-0 flex-col">
          <Topbar
            run={run}
            message={message}
            pending={pending}
            onRunNow={runNow}
            searchRef={searchRef}
            onSearch={focusSearch}
          />
          <main id="main" tabIndex={-1} className="min-h-0 flex-1 overflow-y-auto">
            <Outlet />
          </main>
        </div>
      </div>
      {/* S6.7's leaf (EXPERIENCE §6): the command palette, opened by Cmd/Ctrl+K. */}
      <CommandPalette />
    </>
  );
}

/** Catch-all screen inside the shell (placeholder empty state, AC1). */
export function NotFound() {
  const { t } = useTranslation();
  return <EmptyState title={t("shell:empty.title")} description={t("shell:empty.body")} />;
}
