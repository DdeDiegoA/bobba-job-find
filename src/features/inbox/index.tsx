import { useTranslation } from "react-i18next";
import InboxList from "./list";
import InboxManualAdd from "./manual-add";

/**
 * Inbox container (S6.4 owns it, backlog convention 7): the ranked list plus the
 * leaves the other slices own — today S4.7's Manual Add, behind a disclosure so
 * it does not push the list down. Each later slice replaces its own leaf; the
 * container appends one line for a new panel and is otherwise not edited.
 */
export default function InboxIndex() {
  const { t } = useTranslation("inbox");

  return (
    <section
      className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-6"
      aria-labelledby="inbox-title"
    >
      <header className="flex flex-wrap items-baseline justify-between gap-2">
        <h1 id="inbox-title" className="font-display text-2xl text-fg">
          {t("title")}
        </h1>
      </header>

      <InboxList />

      {/* S4.7's leaf (FR-25): the supported way in for a job found where
          automation is not allowed. */}
      <details className="rounded-card border border-border bg-surface">
        <summary className="cursor-pointer px-4 py-3 text-sm text-fg-muted">
          {t("manual.summary")}
        </summary>
        <InboxManualAdd />
      </details>
    </section>
  );
}
