import { useTranslation } from "react-i18next";

/**
 * The Inbox's loading state (EXPERIENCE §5: skeletons shaped like the final
 * content, never a spinner for a content region). Three card-shaped blocks the
 * height of a real card, announced as busy while they stand in for it.
 */
export function InboxSkeleton({ cards = 3 }: { cards?: number }) {
  const { t } = useTranslation("inbox");
  return (
    <div className="flex flex-col gap-3" aria-busy="true" aria-live="polite">
      <span className="sr-only">{t("loading")}</span>
      {Array.from({ length: cards }, (_, index) => (
        <div
          // biome-ignore lint/suspicious/noArrayIndexKey: the blocks are identical and never reorder
          key={index}
          className="h-28 animate-pulse rounded-card border border-border bg-surface-muted"
        />
      ))}
    </div>
  );
}
