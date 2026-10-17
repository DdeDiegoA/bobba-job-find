import { ChevronDown, ChevronRight, Inbox as InboxIcon } from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { cn } from "@/lib/utils";
import { OfferCard } from "./card";
import {
  cardsIn,
  type Filters,
  filterCards,
  filterOptions,
  hasFilters,
  NO_FILTERS,
  SECTIONS,
  type SectionId,
  SORTS,
  type SortKey,
  sortCards,
  useInboxOffers,
  VERDICT_FILTERS,
  VERDICT_LABELS,
  WORK_MODE_LABELS,
} from "./offers";
import { InboxSkeleton } from "./skeleton";

/**
 * The Inbox list (S6.4, FR-63/FR-64, EXPERIENCE §9.3/§9.11).
 *
 * Six sections — Strong, Good, Moderate, Weak/Poor (collapsed), Closing soon and
 * Excluded — of the same one list the core sends. The five filters and the four
 * sorts of FR-63 narrow and order it here; they change nothing in the core and
 * the core changes nothing here.
 *
 * States (EXPERIENCE §5): skeleton while the first page loads; the previous list
 * stays visible and **dimmed** while a re-run refreshes it; a true empty state
 * only when there are no Offers at all; a filtered-empty state with **Clear
 * filters** when the filters are what emptied it.
 */
export default function InboxList() {
  const { t } = useTranslation("inbox");
  // Errors carry keys from the `common` catalog (AD-3).
  const { t: tError } = useTranslation();
  const inbox = useInboxOffers();
  const [filters, setFilters] = useState<Filters>(NO_FILTERS);
  const [sort, setSort] = useState<SortKey>("rank");
  // FR-63: Weak/Poor starts collapsed; the user opens it (or any other section).
  const [open, setOpen] = useState<SectionId[]>([]);

  const cards = useMemo(() => inbox.data ?? [], [inbox.data]);
  const options = useMemo(() => filterOptions(cards), [cards]);
  const visible = useMemo(
    () => sortCards(filterCards(cards, filters), sort),
    [cards, filters, sort],
  );

  if (inbox.isPending) {
    return <InboxSkeleton />;
  }

  if (inbox.isError && !inbox.data) {
    return (
      <div className="flex flex-col items-start gap-2 p-6" role="alert">
        <p className="text-sm text-danger">
          {isAppError(inbox.error) ? renderAppError(inbox.error, tError) : t("error.load")}
        </p>
        <Button variant="outline" size="sm" onClick={() => void inbox.refetch()}>
          {t("error.retry")}
        </Button>
      </div>
    );
  }

  if (cards.length === 0) {
    return <EmptyState icon={InboxIcon} title={t("empty")} description={t("empty.hint")} />;
  }

  // A re-run keeps the previous list on screen, visibly stale (§5): never blank.
  const stale = inbox.isFetching;

  return (
    <div className="flex min-h-0 flex-col gap-4">
      <FilterBar
        filters={filters}
        onChange={setFilters}
        sort={sort}
        onSort={setSort}
        options={options}
      />

      {visible.length === 0 ? (
        <div className="flex flex-col items-start gap-2 p-6" role="status">
          <p className="text-sm text-fg-muted">{t("filtered.empty")}</p>
          <Button variant="outline" size="sm" onClick={() => setFilters(NO_FILTERS)}>
            {t("filter.clear")}
          </Button>
        </div>
      ) : (
        <div
          className={cn("flex min-h-0 flex-col gap-4 transition-opacity", stale && "opacity-60")}
          aria-busy={stale}
        >
          {SECTIONS.map((section) => {
            const inSection = cardsIn(visible, section.id);
            const expanded = section.collapsed
              ? open.includes(section.id)
              : !open.includes(section.id);
            return (
              <SectionBlock
                key={section.id}
                id={section.id}
                labelKey={section.labelKey}
                count={inSection.length}
                expanded={expanded}
                onToggle={() =>
                  setOpen((current) =>
                    current.includes(section.id)
                      ? current.filter((id) => id !== section.id)
                      : [...current, section.id],
                  )
                }
              >
                <ul className="flex flex-col gap-2" aria-label={t(section.labelKey)}>
                  {inSection.map((card) => (
                    <li key={card.posting_id}>
                      <OfferCard card={card} />
                    </li>
                  ))}
                </ul>
              </SectionBlock>
            );
          })}
        </div>
      )}
    </div>
  );
}

/** One collapsible section: its name, its count and how to open it (NFR-15). */
function SectionBlock({
  id,
  labelKey,
  count,
  expanded,
  onToggle,
  children,
}: {
  id: SectionId;
  labelKey: string;
  count: number;
  expanded: boolean;
  onToggle: () => void;
  children: ReactNode;
}) {
  const { t } = useTranslation("inbox");
  const Icon = expanded ? ChevronDown : ChevronRight;

  return (
    <section aria-labelledby={`inbox-section-${id}`} className="flex flex-col gap-2">
      <h2 id={`inbox-section-${id}`}>
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={expanded}
          className="inline-flex items-center gap-1.5 rounded-lg px-1 py-0.5 font-display text-sm text-fg hover:bg-surface-muted"
        >
          <Icon aria-hidden className="size-4 text-fg-subtle" />
          <span className="text-fg-subtle">
            {t(labelKey)} ({count})
          </span>
        </button>
      </h2>
      {expanded && count > 0 ? children : null}
    </section>
  );
}

/** The filters and the sort of FR-63. A native select per axis, so it is usable
 * by keyboard and announced as a choice box without a custom widget. */
function FilterBar({
  filters,
  onChange,
  sort,
  onSort,
  options,
}: {
  filters: Filters;
  onChange: (filters: Filters) => void;
  sort: SortKey;
  onSort: (sort: SortKey) => void;
  options: { sources: string[]; workModes: string[]; langs: string[] };
}) {
  const { t } = useTranslation("inbox");

  return (
    <div className="flex flex-wrap items-end gap-3 rounded-card border border-border bg-surface p-3">
      <Select
        label={t("filter.source")}
        value={filters.source ?? ""}
        onChange={(value) => onChange({ ...filters, source: value || null })}
        options={options.sources}
        anyLabel={t("filter.any")}
      />
      <Select
        label={t("filter.work_mode")}
        value={filters.workMode ?? ""}
        onChange={(value) => onChange({ ...filters, workMode: value || null })}
        options={options.workModes}
        labelOf={(id) => {
          const key = WORK_MODE_LABELS[id];
          return key ? t(key) : id;
        }}
        anyLabel={t("filter.any")}
      />
      <Select
        label={t("filter.language")}
        value={filters.lang ?? ""}
        onChange={(value) => onChange({ ...filters, lang: value || null })}
        options={options.langs}
        anyLabel={t("filter.any")}
      />
      <Select
        label={t("filter.verdict")}
        value={filters.verdict ?? ""}
        onChange={(value) => onChange({ ...filters, verdict: value || null })}
        options={[...VERDICT_FILTERS]}
        labelOf={(id) => (VERDICT_LABELS[id] ? t(VERDICT_LABELS[id]) : id)}
        anyLabel={t("filter.any")}
      />
      <label className="inline-flex items-center gap-2 text-sm text-fg">
        <input
          type="checkbox"
          checked={filters.salaryKnown}
          onChange={(event) => onChange({ ...filters, salaryKnown: event.target.checked })}
        />
        {t("filter.salary_known")}
      </label>

      <label className="ml-auto inline-flex items-center gap-2 text-sm text-fg-muted">
        {t("sort.label")}
        <select
          value={sort}
          onChange={(event) => onSort(event.target.value as SortKey)}
          className="h-9 rounded-xl border border-border bg-surface px-2 text-sm text-fg"
        >
          {SORTS.map((option) => (
            <option key={option.key} value={option.key}>
              {t(option.labelKey)}
            </option>
          ))}
        </select>
      </label>

      {hasFilters(filters) ? (
        <Button variant="ghost" size="sm" onClick={() => onChange(NO_FILTERS)}>
          {t("filter.clear")}
        </Button>
      ) : null}
    </div>
  );
}

/** One labelled choice box; an empty value is the "any" option. */
function Select({
  label,
  value,
  onChange,
  options,
  labelOf,
  anyLabel,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: string[];
  labelOf?: (id: string) => string;
  anyLabel: string;
}) {
  return (
    <label className="flex flex-col gap-1 text-xs text-fg-muted">
      {label}
      <select
        value={value}
        onChange={(event) => onChange(event.target.value)}
        className="h-9 min-w-32 rounded-xl border border-border bg-surface px-2 text-sm text-fg"
      >
        <option value="">{anyLabel}</option>
        {options.map((option) => (
          <option key={option} value={option}>
            {labelOf ? labelOf(option) : option}
          </option>
        ))}
      </select>
    </label>
  );
}
