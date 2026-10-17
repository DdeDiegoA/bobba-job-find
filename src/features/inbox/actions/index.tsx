import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import type { OfferCardView, SuggestionView } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { useBulkStore } from "./bulk-store";
import { useSuggestionStore } from "./suggestion-store";
import {
  useDecideSuggestion,
  useDismissOffer,
  useOverrideVeto,
  useSaveOffer,
  useUndoDismiss,
} from "./use-actions";

/**
 * Inbox actions (S6.6, FR-54, FR-66, FR-67, EXPERIENCE §9.3).
 *
 * Three things the Inbox lets the user do without opening an Offer:
 *
 * * **Save** — an Application in `Saved`, through the core (FR-66).
 * * **Dismiss** — with an optional reason, and a 6-second Undo on the toast
 *   (FR-66). The reason is the signal FR-67 counts.
 * * **Override veto** — only on an Excluded Offer (FR-54): the Gate stays on
 *   record, the Offer is shown again.
 *
 * The Preferences change a third dismissal suggests (FR-67 AC3) is decided by
 * the core and drawn by [`SuggestionBanner`]; bulk dismissal is
 * [`InboxBulkBar`].
 */

/** The reasons of FR-66, in menu order; the labels live in the catalogue. */
const REASONS = [
  "not_interested",
  "not_eligible",
  "salary",
  "company",
  "location",
  "other",
] as const;

/** FR-66: the Undo offer lives this long (the AC's 6 seconds). */
export const UNDO_WINDOW_MS = 6000;

/** One card's actions: Save, Dismiss (with a reason) and Override when vetoed. */
export function OfferActions({ card }: { card: OfferCardView }) {
  const { t } = useTranslation("inbox_actions");
  const { t: tError } = useTranslation();
  const [menuOpen, setMenuOpen] = useState(false);
  const save = useSaveOffer();
  const dismiss = useDismissOffer();
  const undo = useUndoDismiss();
  const override = useOverrideVeto();
  const bulkActive = useBulkStore((state) => state.active);
  const picked = useBulkStore((state) => state.selected.includes(card.posting_id));
  const toggle = useBulkStore((state) => state.toggle);

  // A typed error renders its own key; anything else falls back to the local
  // sentence at the call site (so both keys are in the catalogue).
  const failure = (error: unknown) => (isAppError(error) ? renderAppError(error, tError) : null);

  const onSave = () => {
    save.mutate(card.posting_id, {
      onSuccess: () => toast(t("save.done")),
      onError: (error) => toast.error(failure(error) ?? t("save.error")),
    });
  };

  const onDismiss = (reason: string | null) => {
    setMenuOpen(false);
    dismiss.mutate(
      { postingIds: [card.posting_id], reason },
      {
        onSuccess: () => {
          toast(t("dismiss.done"), {
            duration: UNDO_WINDOW_MS,
            action: {
              label: t("undo"),
              onClick: () =>
                undo.mutate(card.posting_id, {
                  onSuccess: () => toast(t("undo.done")),
                  onError: (error) => toast.error(failure(error) ?? t("undo.error")),
                }),
            },
          });
        },
        onError: (error) => toast.error(failure(error) ?? t("dismiss.error")),
      },
    );
  };

  const onOverride = () => {
    override.mutate(card.posting_id, {
      onSuccess: () => toast(t("override.done")),
      onError: (error) => toast.error(failure(error) ?? t("override.error")),
    });
  };

  const vetoed = card.exclusion !== null && !card.overridden;

  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {bulkActive ? (
        <label className="flex items-center gap-1.5 text-xs text-fg-muted">
          <input
            type="checkbox"
            className="size-4 accent-[var(--color-focus)]"
            checked={picked}
            onChange={() => toggle(card.posting_id)}
            aria-label={t("bulk.pick", { title: card.title || t("inbox:card.untitled") })}
          />
        </label>
      ) : null}

      <Button size="sm" variant="outline" onClick={onSave} disabled={save.isPending}>
        {t("save")}
      </Button>

      <div className="relative">
        <Button
          size="sm"
          variant="ghost"
          aria-expanded={menuOpen}
          aria-haspopup="menu"
          onClick={() => setMenuOpen((open) => !open)}
          disabled={dismiss.isPending}
        >
          {t("dismiss")}
        </Button>
        {menuOpen ? (
          <div
            role="menu"
            className="absolute right-0 z-10 mt-1 flex w-56 flex-col gap-1 rounded-card border border-border bg-surface p-2 shadow-lg"
            aria-label={t("dismiss.reason")}
          >
            <p className="px-1 text-xs text-fg-subtle">{t("dismiss.reason")}</p>
            {REASONS.map((reason) => (
              <Button
                key={reason}
                size="sm"
                variant="ghost"
                role="menuitem"
                className="justify-start"
                onClick={() => onDismiss(reason)}
              >
                {t(`inbox_actions:reason.${reason}`)}
              </Button>
            ))}
            <Button size="sm" variant="outline" role="menuitem" onClick={() => onDismiss(null)}>
              {t("dismiss.without_reason")}
            </Button>
          </div>
        ) : null}
      </div>

      {vetoed ? (
        <Button
          size="sm"
          variant="ghost"
          title={t("override.hint")}
          onClick={onOverride}
          disabled={override.isPending}
        >
          {t("override")}
        </Button>
      ) : null}
    </div>
  );
}

/** Bulk dismissal (FR-66): pick cards, then dismiss them in one call. */
export function InboxBulkBar({ cards }: { cards: readonly OfferCardView[] }) {
  const { t } = useTranslation("inbox_actions");
  const { t: tError } = useTranslation();
  const active = useBulkStore((state) => state.active);
  const selected = useBulkStore((state) => state.selected);
  const start = useBulkStore((state) => state.start);
  const stop = useBulkStore((state) => state.stop);
  const selectAll = useBulkStore((state) => state.selectAll);
  const clear = useBulkStore((state) => state.clear);
  const forget = useBulkStore((state) => state.forget);
  const dismiss = useDismissOffer();

  if (!active) {
    return (
      <div className="flex justify-end">
        <Button size="sm" variant="ghost" onClick={start}>
          {t("bulk.enter")}
        </Button>
      </div>
    );
  }

  const dismissPicked = (reason: string | null) => {
    const postingIds = [...selected];
    dismiss.mutate(
      { postingIds, reason },
      {
        onSuccess: (outcome) => {
          forget(outcome.dismissed);
          toast(t("bulk.done", { count: outcome.dismissed.length }));
          if (selected.length === outcome.dismissed.length) stop();
        },
        onError: (error) =>
          toast.error(isAppError(error) ? renderAppError(error, tError) : t("dismiss.error")),
      },
    );
  };

  return (
    <div
      className="flex flex-wrap items-center gap-2 rounded-card border border-border bg-surface px-3 py-2"
      role="toolbar"
      aria-label={t("bulk.aria")}
    >
      <span className="text-sm text-fg-muted">{t("bulk.count", { count: selected.length })}</span>
      <Button
        size="sm"
        variant="ghost"
        onClick={() => selectAll(cards.map((card) => card.posting_id))}
      >
        {t("bulk.select_all")}
      </Button>
      <Button size="sm" variant="ghost" onClick={clear} disabled={selected.length === 0}>
        {t("bulk.clear")}
      </Button>
      <div className="ml-auto flex flex-wrap items-center gap-1.5">
        {REASONS.map((reason) => (
          <Button
            key={reason}
            size="sm"
            variant="outline"
            disabled={selected.length === 0 || dismiss.isPending}
            onClick={() => dismissPicked(reason)}
          >
            {t(`inbox_actions:reason.${reason}`)}
          </Button>
        ))}
        <Button
          size="sm"
          variant="ghost"
          disabled={selected.length === 0 || dismiss.isPending}
          onClick={() => dismissPicked(null)}
        >
          {t("bulk.dismiss", { count: selected.length })}
        </Button>
        <Button size="sm" variant="ghost" onClick={stop}>
          {t("bulk.exit")}
        </Button>
      </div>
    </div>
  );
}

/**
 * The Preferences change Boba suggests after the third dismissal of a kind
 * (FR-67 AC3). Accepting writes the change; ignoring stops Boba asking again.
 */
export function SuggestionBanner() {
  const { t } = useTranslation("inbox_actions");
  const { t: tError } = useTranslation();
  const current = useSuggestionStore((state) => state.current);
  const clear = useSuggestionStore((state) => state.clear);
  const decide = useDecideSuggestion();

  if (!current) return null;

  const decideNow = (accept: boolean) => {
    decide.mutate(
      { id: current.id, accept, company: current.company },
      {
        onSuccess: () => toast(accept ? t("suggestion.accepted") : t("suggestion.ignored")),
        onError: (error) =>
          toast.error(
            (isAppError(error) ? renderAppError(error, tError) : null) ?? t("suggestion.error"),
          ),
      },
    );
  };

  return (
    <aside
      className="flex flex-wrap items-center gap-3 rounded-card border border-band-good/40 bg-surface px-4 py-3"
      aria-label={t("suggestion.aria")}
    >
      <p className="text-sm text-fg">{sentence(current, t)}</p>
      <div className="ml-auto flex items-center gap-1.5">
        <Button size="sm" onClick={() => decideNow(true)} disabled={decide.isPending}>
          {t("suggestion.accept")}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => decideNow(false)}
          disabled={decide.isPending}
        >
          {t("suggestion.ignore")}
        </Button>
        <Button size="sm" variant="ghost" onClick={clear} disabled={decide.isPending}>
          {t("suggestion.later")}
        </Button>
      </div>
    </aside>
  );
}

/** The sentence for one suggestion; both kinds are FR-67's own examples. */
function sentence(
  suggestion: SuggestionView,
  t: (key: string, values?: Record<string, unknown>) => string,
): string {
  if (suggestion.kind === "exclude_company") {
    return t("suggestion.company", {
      count: suggestion.count,
      company: suggestion.company ?? "",
    });
  }
  return t("suggestion.onsite", { count: suggestion.count });
}
