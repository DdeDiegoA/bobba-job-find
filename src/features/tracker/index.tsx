import { type KeyboardEvent as ReactKeyboardEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { Toaster } from "sonner";
import { Board } from "./board/board";
import TrackerNudges from "./nudges";
import TrackerStats from "./stats";

/**
 * Tracker — the kanban board of Applications (S9.1, EXPERIENCE §9.6) and the
 * funnel stats (S9.9, FR-97). The container is owned by this slice (S1.5 left a
 * stub); the board is in `board/` and the stats in `stats/`.
 *
 * The `<Toaster>` lives here because Undo is offered as a toast right after a
 * move (AC2); no other screen has mounted one yet.
 */
export default function TrackerIndex() {
  const { t } = useTranslation("tracker");
  const { t: tStats } = useTranslation("audit_stats");
  const [view, setView] = useState<"board" | "stats">("board");
  const tabs = [
    { id: "board" as const, label: tStats("audit_stats:tabs.board") },
    { id: "stats" as const, label: tStats("audit_stats:tabs.stats") },
  ];
  const tabId = (id: string) => `tracker-tab-${id}`;
  // One region shows the selected view, so both tabs point at the same panel
  // and it is named by whichever tab is selected.
  const panelId = "tracker-panel";

  // A11Y-8: a real tablist — roving tabindex and arrow-key selection.
  const onTabKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>, index: number) => {
    const step =
      event.key === "ArrowRight"
        ? 1
        : event.key === "ArrowLeft"
          ? -1
          : event.key === "Home"
            ? -index
            : event.key === "End"
              ? tabs.length - 1 - index
              : 0;
    if (step === 0 && event.key !== "Home" && event.key !== "End") return;
    event.preventDefault();
    const next = tabs[(index + step + tabs.length) % tabs.length];
    setView(next.id);
    document.getElementById(tabId(next.id))?.focus();
  };

  return (
    <section className="flex min-h-0 flex-1 flex-col gap-4 p-6" aria-labelledby="tracker-title">
      <div className="flex items-end justify-between gap-3">
        <h1 id="tracker-title" className="font-display text-2xl text-fg">
          {t("title")}
        </h1>
        <div role="tablist" aria-label={tStats("audit_stats:tabs.aria")} className="flex gap-1">
          {tabs.map((tab, index) => (
            <button
              key={tab.id}
              id={tabId(tab.id)}
              type="button"
              role="tab"
              aria-selected={view === tab.id}
              aria-controls={panelId}
              tabIndex={view === tab.id ? 0 : -1}
              className={
                view === tab.id
                  ? "rounded-xl bg-primary-soft px-3 py-1.5 text-sm text-on-primary-soft"
                  : "rounded-xl px-3 py-1.5 text-sm text-fg-muted hover:bg-surface-muted hover:text-fg"
              }
              onClick={() => setView(tab.id)}
              onKeyDown={(event) => onTabKeyDown(event, index)}
            >
              {tab.label}
            </button>
          ))}
        </div>
      </div>

      <div
        role="tabpanel"
        id={panelId}
        aria-labelledby={tabId(view)}
        className="flex min-h-0 flex-1 flex-col gap-4"
      >
        {view === "board" ? (
          <>
            {/* S9.5: follow-ups for applications that went quiet (FR-95). */}
            <TrackerNudges />
            <Board />
          </>
        ) : (
          <TrackerStats />
        )}
      </div>
      <Toaster />
    </section>
  );
}
