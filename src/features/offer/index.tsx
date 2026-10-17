import { useTranslation } from "react-i18next";
import { Link, useParams } from "react-router";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { uiLanguage, useOfferDetail } from "./detail";
import { DualScore } from "./dual-score";
import { Breakdown, Channels, Gates, Header, Narration, Posting, Sources } from "./sections";

/**
 * The Offer detail (S6.5, FR-65, EXPERIENCE §9.4).
 *
 * One screen, top to bottom: the header, the DualScore (whose axes scroll to
 * their factor group, AC3), the Gates with their quotes, the Explanation, the
 * factor breakdown, the posting itself, its Sources and merged copies, and the
 * Apply Channels with the link to the original.
 *
 * States (EXPERIENCE §5): a skeleton while the first read lands; the typed error
 * with a retry; and — when no Offer is selected, or the id names none — a calm
 * empty state rather than an error.
 */
export default function OfferDetail() {
  const { t, i18n } = useTranslation("offer");
  const { t: tError } = useTranslation();
  const { postingId } = useParams<{ postingId: string }>();
  const lang = uiLanguage(i18n.language);
  const detail = useOfferDetail(postingId ?? "", lang);

  if (!postingId) {
    return (
      <section className="p-6">
        <EmptyState title={t("empty.title")} description={t("empty.hint")} />
      </section>
    );
  }

  if (detail.isPending) {
    return (
      <section className="flex flex-col gap-4 p-6" aria-busy="true">
        <div className="h-8 w-2/3 animate-pulse rounded bg-surface" />
        <div className="h-24 animate-pulse rounded-card bg-surface" />
        <div className="h-40 animate-pulse rounded-card bg-surface" />
      </section>
    );
  }

  if (detail.isError || !detail.data) {
    return (
      <section className="flex flex-col items-start gap-2 p-6" role="alert">
        <p className="text-sm text-danger">
          {isAppError(detail.error) ? renderAppError(detail.error, tError) : t("load_error")}
        </p>
        <Button variant="outline" size="sm" onClick={() => void detail.refetch()}>
          {t("retry")}
        </Button>
        <Link to="/inbox" className="text-sm text-fg-muted underline-offset-2 hover:underline">
          {t("back")}
        </Link>
      </section>
    );
  }

  const view = detail.data;
  return (
    <section className="flex flex-col gap-6 p-6" aria-labelledby="offer-title">
      <nav className="text-sm">
        <Link to="/inbox" className="text-fg-muted underline-offset-2 hover:underline">
          {t("back")}
        </Link>
      </nav>

      <Header detail={view} locale={i18n.language} />
      <DualScore detail={view} />
      <Gates detail={view} />
      <Narration detail={view} />
      <Breakdown detail={view} />
      <Posting detail={view} />
      <Sources detail={view} />
      <Channels detail={view} />
    </section>
  );
}
