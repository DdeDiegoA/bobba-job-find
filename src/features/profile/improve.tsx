import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { Button } from "@/components/ui/button";
import { useCapability } from "@/lib/capabilities";
import { currentLocale } from "@/lib/i18n";
import type { FactView, ProfileSnapshot, ProposalView } from "@/lib/ipc/bindings";
import { commands } from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";
import { profileKey } from "./use-profile";

/** Where the user picks or changes the Agent CLI (FX7's Settings page). */
const AI_ENGINE_PATH = "/settings/ai-engine";

/** The variants the user accepted, so the form's own fields match the save. */
export interface ImprovedText {
  en?: string | null;
  es?: string | null;
}

/**
 * Suggest an improvement to one fact (FX10b, issue 5).
 *
 * The suggestion comes from the `improve_fact` agent run (FX10a) — the same
 * runtime and lockdown gate onboarding uses — and arrives as a **pending
 * proposal**. The user sees the fact's current text and the rewrite side by
 * side, verbatim as plain text, and nothing is saved until they accept that
 * one suggestion. Reject discards it.
 *
 * This is the only surface that calls `profile_improve_confirm` (FX10a review
 * R5): every accept names exactly the suggestion the user was shown.
 */
export function ImproveFact({
  fact,
  onConfirmed,
}: {
  fact: FactView;
  /** The accepted text, so the form's own fields match the saved version. */
  onConfirmed: (changed: ImprovedText) => void;
}) {
  const { t } = useTranslation("profile");
  const { t: tError } = useTranslation();
  // The improvement run is profile work gated on the chosen engine, like the
  // gap interview that writes the same kind of proposal (FX8/FX9).
  const access = useCapability("profile_gaps");
  const queryClient = useQueryClient();
  const [suggestions, setSuggestions] = useState<ProposalView[]>([]);
  const [removed, setRemoved] = useState<string[]>([]);
  /** The last run returned no suggestion (review F2: never after a review). */
  const [empty, setEmpty] = useState(false);
  const [errorText, setErrorText] = useState<string | null>(null);
  // Review F1: a reviewed card unmounts; focus must land on the next one, or
  // back on the action when none is left — never on the document body.
  // `undefined` = no request; `null` = the Suggest button; an id = that card.
  const [focusAfter, setFocusAfter] = useState<string | null | undefined>(undefined);

  const start = useMutation({
    mutationFn: () => commands.profileImproveStart(fact.item_id, currentLocale()).then(unwrap),
  });
  const confirm = useMutation({
    mutationFn: (id: string) => commands.profileImproveConfirm(id, null).then(unwrap),
    onSuccess: (snapshot: ProfileSnapshot) => queryClient.setQueryData(profileKey, snapshot),
  });
  const discard = useMutation({
    mutationFn: (id: string) => commands.profileImproveDiscard(id).then(unwrap),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: profileKey });
    },
  });

  const busy = start.isPending || confirm.isPending || discard.isPending;
  const pending = suggestions.filter((item) => !removed.includes(item.id));
  const fail = (error: unknown) =>
    setErrorText(isAppError(error) ? renderAppError(error, tError) : tError("errors.unknown"));

  useEffect(() => {
    if (focusAfter === undefined) return;
    const target =
      focusAfter === null
        ? document.getElementById(suggestId(fact.item_id))
        : document.getElementById(acceptId(focusAfter));
    target?.focus();
    setFocusAfter(undefined);
  }, [focusAfter, fact.item_id]);

  /** The core or the user dropped it; stop listing it either way. */
  const drop = (id: string) => {
    // The next card takes focus; the previous one when the last card goes;
    // the action itself when the list is now empty.
    const order = pending.map((item) => item.id);
    const at = order.indexOf(id);
    setFocusAfter(order[at + 1] ?? order[at - 1] ?? null);
    setRemoved((current) => [...current, id]);
  };

  // This action's own copy, not the shared banner: the same form may already
  // show the FX8 draft's notice, and two identical blocks would read as one.
  const unavailable =
    access.reason === "cli_unsupported"
      ? t("improve.unsupported")
      : tError("errors.profile_improve.no_engine");

  return (
    <section aria-label={t("improve.aria")} className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <Button
          id={suggestId(fact.item_id)}
          type="button"
          size="sm"
          variant="outline"
          disabled={!access.available || busy}
          onClick={() => {
            setErrorText(null);
            setEmpty(false);
            start.mutate(undefined, {
              onSuccess: (outcome) => {
                // Append, never replace: an earlier suggestion stays actionable.
                setSuggestions((current) => mergeSuggestions(current, outcome.proposals));
                setEmpty(outcome.proposals.length === 0);
              },
              onError: fail,
            });
          }}
        >
          {start.isPending ? t("improve.running") : t("improve.suggest")}
        </Button>
      </div>

      {/* FX8 review F3: an unavailable action always says why. */}
      {!access.available && !access.isLoading ? (
        <p className="text-sm text-fg-muted" role="status">
          {unavailable}{" "}
          <Link to={AI_ENGINE_PATH} className="underline">
            {t("improve.openSettings")}
          </Link>
        </p>
      ) : null}

      {errorText ? (
        <p className="text-sm text-danger" role="alert">
          {errorText}
        </p>
      ) : null}
      {start.isPending ? (
        <p className="text-sm text-fg-muted" role="status">
          {t("improve.running")}
        </p>
      ) : null}
      {empty ? (
        <p className="text-sm text-fg-muted" role="status">
          {t("improve.empty")}
        </p>
      ) : null}

      {pending.length > 0 ? (
        <ul className="flex flex-col gap-3">
          {pending.map((suggestion) => (
            <ImprovementCard
              key={suggestion.id}
              fact={fact}
              suggestion={suggestion}
              busy={busy}
              onAccept={() =>
                confirm.mutate(suggestion.id, {
                  onSuccess: () => {
                    drop(suggestion.id);
                    onConfirmed(changed(fact, suggestion));
                  },
                  onError: fail,
                })
              }
              onReject={() =>
                discard.mutate(suggestion.id, {
                  onSuccess: () => drop(suggestion.id),
                  onError: fail,
                })
              }
            />
          ))}
        </ul>
      ) : null}
    </section>
  );
}

/**
 * One suggestion, with the fact's current text next to it (AC1). Accept and
 * Reject name this suggestion only (FX10a review R5).
 */
function ImprovementCard({
  fact,
  suggestion,
  busy,
  onAccept,
  onReject,
}: {
  fact: FactView;
  suggestion: ProposalView;
  busy: boolean;
  onAccept: () => void;
  onReject: () => void;
}) {
  const { t } = useTranslation("profile");
  // Review F3: "Accept" and "Reject" repeat on every card, so both name the
  // suggested text they act on.
  const describedBy = suggestionTextId(suggestion.id);

  return (
    <li className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-4">
      <h3 className="font-medium text-fg">{t("improve.suggestion")}</h3>
      {suggestion.untrusted_origin ? (
        <p className="text-sm text-warning">{t("drafts.untrusted")}</p>
      ) : null}
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="flex flex-col gap-1">
          <p className="text-xs uppercase tracking-wide text-fg-subtle">{t("improve.current")}</p>
          {fact.text_en ? (
            <p className="whitespace-pre-wrap break-words text-fg">{fact.text_en}</p>
          ) : null}
          {fact.text_es ? (
            <p className="whitespace-pre-wrap break-words text-fg-muted">{fact.text_es}</p>
          ) : null}
        </div>
        <div id={describedBy} className="flex flex-col gap-1">
          <p className="text-xs uppercase tracking-wide text-fg-subtle">{t("improve.suggested")}</p>
          {suggestion.text_en ? (
            <p className="whitespace-pre-wrap break-words text-fg">{suggestion.text_en}</p>
          ) : null}
          {suggestion.text_es ? (
            <p className="whitespace-pre-wrap break-words text-fg-muted">{suggestion.text_es}</p>
          ) : null}
        </div>
      </div>
      <div className="flex flex-wrap gap-2">
        <Button
          id={acceptId(suggestion.id)}
          type="button"
          size="sm"
          aria-describedby={describedBy}
          disabled={busy}
          onClick={onAccept}
        >
          {t("improve.accept")}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          aria-describedby={describedBy}
          disabled={busy}
          onClick={onReject}
        >
          {t("improve.reject")}
        </Button>
      </div>
    </li>
  );
}

/** The variants the suggestion changed, so the form mirrors the saved fact. */
function changed(fact: FactView, suggestion: ProposalView): ImprovedText {
  const result: ImprovedText = {};
  if (suggestion.text_en !== fact.text_en) result.en = suggestion.text_en;
  if (suggestion.text_es !== fact.text_es) result.es = suggestion.text_es;
  return result;
}

/** The id of the action that starts a run for one fact. */
function suggestId(itemId: string): string {
  return `improve-suggest-${itemId}`;
}

/** The id of the card's Accept button: where focus lands after a review. */
function acceptId(suggestionId: string): string {
  return `improve-accept-${suggestionId}`;
}

/** The id of a suggestion's text: what its Accept and Reject describe. */
function suggestionTextId(suggestionId: string): string {
  return `improve-suggested-${suggestionId}`;
}

/** Keep suggestions in arrival order without duplicating an id. */
function mergeSuggestions(current: ProposalView[], next: ProposalView[]): ProposalView[] {
  const ids = new Set(current.map((item) => item.id));
  return [...current, ...next.filter((item) => !ids.has(item.id))];
}
