import { useState } from "react";
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

  return (
    <section className="flex min-h-0 flex-1 flex-col gap-4 p-6" aria-labelledby="tracker-title">
      <div className="flex items-end justify-between gap-3">
        <h1 id="tracker-title" className="font-display text-2xl text-fg">
          {t("title")}
        </h1>
        <div role="tablist" aria-label={tStats("audit_stats:tabs.aria")} className="flex gap-1">
          {tabs.map((tab) => (
            <button
              key={tab.id}
              type="button"
              role="tab"
              aria-selected={view === tab.id}
              className={
                view === tab.id
                  ? "rounded-xl bg-primary-soft px-3 py-1.5 text-sm text-on-primary-soft"
                  : "rounded-xl px-3 py-1.5 text-sm text-fg-muted hover:bg-surface-muted hover:text-fg"
              }
              onClick={() => setView(tab.id)}
            >
              {tab.label}
            </button>
          ))}
        </div>
      </div>

      {view === "board" ? (
        <>
          {/* S9.5: follow-ups for applications that went quiet (FR-95). */}
          <TrackerNudges />
          <Board />
        </>
      ) : (
        <TrackerStats />
      )}
      <Toaster />
    </section>
  );
}
