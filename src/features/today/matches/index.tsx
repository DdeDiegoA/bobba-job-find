import { ArrowRight, Sparkles } from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { Button } from "@/components/ui/button";
import {
  bandCounts,
  isExcluded,
  sortCards,
  useInboxOffers,
  VERDICT_LABELS,
  verdictId,
  WORK_MODE_LABELS,
} from "@/features/inbox/list/offers";
import { isAppError, renderAppError } from "@/lib/ipc/errors";

/**
 * Today's matches (S6.4, EXPERIENCE §9.2): how many Offers landed in the three
 * bands worth a look today, a way into the Inbox, and the closest few on top.
 *
 * It reads the same list the Inbox reads (`useInboxOffers`, one cached query) so
 * the two screens can never disagree about a number. A re-run dims the panel
 * rather than blanking it (§5), and a first run shows a skeleton.
 */

/** How many Offers Today puts on top. §9.2 shows the summary; three is the
 * "on top" list, small enough to stay a glance. */
const TOP_MATCHES = 3;

export default function TodayMatches() {
  const { t } = useTranslation("today");
  const { t: tError } = useTranslation();
  const inbox = useInboxOffers();

  const cards = useMemo(() => inbox.data ?? [], [inbox.data]);
  const counts = useMemo(() => bandCounts(cards), [cards]);
  const top = useMemo(
    () =>
      sortCards(cards, "rank")
        .filter((card) => !isExcluded(card) && !card.dismissed && card.rank !== null)
        .slice(0, TOP_MATCHES),
    [cards],
  );

  if (inbox.isPending) {
    return (
      <section aria-busy="true" aria-label={t("matches.title")}>
        <span className="sr-only">{t("matches.loading")}</span>
        <div className="h-24 animate-pulse rounded-card border border-border bg-surface-muted" />
      </section>
    );
  }

  if (inbox.isError && !inbox.data) {
    return (
      <section aria-label={t("matches.title")} role="alert" className="flex flex-col gap-2">
        <p className="text-sm text-danger">
          {isAppError(inbox.error) ? renderAppError(inbox.error, tError) : t("matches.error.load")}
        </p>
        <Button variant="outline" size="sm" onClick={() => void inbox.refetch()}>
          {t("matches.retry")}
        </Button>
      </section>
    );
  }

  const ranked = counts.strong + counts.good + counts.moderate + counts.weak_poor;

  return (
    <section
      aria-labelledby="today-matches-title"
      className={`flex flex-col gap-3 transition-opacity ${inbox.isFetching ? "opacity-60" : ""}`}
    >
      <h2 id="today-matches-title" className="font-display text-lg text-fg">
        {t("matches.title")}
      </h2>

      {ranked === 0 ? (
        <div className="flex flex-col items-start gap-1 rounded-card border border-border bg-surface p-4">
          <p className="text-sm text-fg">{t("matches.empty")}</p>
          <p className="text-sm text-fg-muted">{t("matches.empty.hint")}</p>
        </div>
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-3 rounded-card border border-border bg-surface p-4">
            <ul className="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
              {(["strong", "good", "moderate"] as const).map((band) => (
                <li key={band} className="inline-flex items-baseline gap-1.5">
                  <span className="font-display text-lg text-fg">{counts[band]}</span>
                  <span className="text-fg-muted">{t(`today:band.${band}`)}</span>
                </li>
              ))}
            </ul>
            <Link to="/inbox" className="ml-auto">
              <Button variant="outline" size="sm">
                {t("matches.open_inbox")}
                <ArrowRight aria-hidden className="size-3.5" />
              </Button>
            </Link>
          </div>

          {top.length > 0 ? (
            <div className="flex flex-col gap-2">
              <h3 className="text-xs font-medium uppercase tracking-wide text-fg-subtle">
                {t("matches.top")}
              </h3>
              <ul className="flex flex-col gap-2">
                {top.map((card) => {
                  const verdict = verdictId(card);
                  return (
                    <li key={card.posting_id}>
                      <Link
                        to={`/offer/${card.posting_id}`}
                        className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-card border border-border bg-surface p-3 text-sm hover:bg-surface-muted"
                      >
                        <span className="min-w-0 truncate font-medium text-fg">{card.title}</span>
                        <span className="min-w-0 truncate text-fg-muted">{card.company}</span>
                        <span className="text-fg-subtle">
                          {WORK_MODE_LABELS[card.work_mode]
                            ? t(WORK_MODE_LABELS[card.work_mode])
                            : card.work_mode}
                        </span>
                        <span className="ml-auto inline-flex items-center gap-1 text-fg-muted">
                          {t("matches.fit", { fit: Math.round(card.fit ?? 0) })}
                          {verdict ? (
                            <span className="inline-flex items-center gap-1 text-fg">
                              {VERDICT_LABELS[verdict] ? t(VERDICT_LABELS[verdict]) : null}
                              {card.rank !== null ? Math.round(card.rank) : null}
                            </span>
                          ) : null}
                        </span>
                      </Link>
                    </li>
                  );
                })}
              </ul>
            </div>
          ) : (
            <p className="inline-flex items-center gap-1.5 text-sm text-fg-muted">
              <Sparkles aria-hidden className="size-4" />
              {t("matches.ranking_soon")}
            </p>
          )}
        </>
      )}
    </section>
  );
}
