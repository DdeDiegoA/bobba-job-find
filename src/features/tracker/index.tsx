import { useTranslation } from "react-i18next";
import { Toaster } from "sonner";
import { Board } from "./board/board";
import TrackerNudges from "./nudges";

/**
 * Tracker — the kanban board of Applications (S9.1, EXPERIENCE §9.6). The
 * container is owned by this slice (S1.5 left a stub); the board is in `board/`.
 *
 * The `<Toaster>` lives here because Undo is offered as a toast right after a
 * move (AC2); no other screen has mounted one yet.
 */
export default function TrackerIndex() {
  const { t } = useTranslation("tracker");

  return (
    <section className="flex min-h-0 flex-1 flex-col gap-4 p-6" aria-labelledby="tracker-title">
      <h1 id="tracker-title" className="font-display text-2xl text-fg">
        {t("title")}
      </h1>
      {/* S9.5: follow-ups for applications that went quiet (FR-95). */}
      <TrackerNudges />
      <Board />
      <Toaster />
    </section>
  );
}
