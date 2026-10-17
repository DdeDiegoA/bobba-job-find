import { useTranslation } from "react-i18next";
import TodayMatches from "./matches";
import TodayOutbox from "./outbox";
import TodayRunReport from "./run-report";

/**
 * Today (S6.4 owns the container, backlog convention 7). The sitemap (§2.2) puts
 * the run digest, the closest matches, Needs you and the Outbox here; each of
 * those is a leaf its own slice owns, and each appends one line to
 * [`TODAY_PANELS`] when its real UI lands. This slice builds the matches panel.
 */
const TODAY_PANELS: readonly { id: string; Component: () => React.JSX.Element }[] = [
  { id: "matches", Component: TodayMatches },
  { id: "outbox", Component: TodayOutbox },
  { id: "run-report", Component: TodayRunReport },
];

export default function TodayIndex() {
  const { t } = useTranslation("today");

  return (
    <section
      className="flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto p-6"
      aria-labelledby="today-title"
    >
      <h1 id="today-title" className="font-display text-2xl text-fg">
        {t("title")}
      </h1>

      {TODAY_PANELS.map((panel) => (
        <panel.Component key={panel.id} />
      ))}
    </section>
  );
}
