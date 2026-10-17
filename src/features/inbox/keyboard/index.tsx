import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { UNDO_WINDOW_MS } from "../actions";
import { useDismissOffer, useSaveOffer, useUndoDismiss } from "../actions/use-actions";
import { useInboxOffers, VERDICT_LABELS, verdictId } from "../list/offers";
import {
  focusCard,
  focusedPostingId,
  focusNeighbour,
  moveCursor,
  postingIdOf,
  rememberCard,
  rememberedCard,
} from "./dom";

/** A field the user is typing in: the triage keys leave it alone. */
function isEditable(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el || typeof el.tagName !== "string") return false;
  return (
    el.tagName === "INPUT" ||
    el.tagName === "TEXTAREA" ||
    el.tagName === "SELECT" ||
    el.isContentEditable
  );
}

/** `Cmd` on macOS, `Ctrl` elsewhere, for the hint line. */
function modKey(): string {
  return /Mac|iPhone|iPad/.test(navigator.platform ?? "") ? "⌘" : "Ctrl";
}

/**
 * Keyboard triage for the Inbox (S6.7, FR-68, EXPERIENCE §6).
 *
 * `J`/`K` move the cursor card by card, `S` saves, `D` (or `X`, the DESIGN
 * §10.4 key) dismisses with a 6-second Undo, `Enter` opens the focused card
 * (its link already does that) and `Cmd/Ctrl+Z` restores the last dismissal.
 * Focus is visible on every card and comes back to where it was after a visit
 * to an Offer.
 *
 * Everything the keys do goes through S6.6's actions, so the core stays the one
 * that decides. The live region below announces the Verdict as text (NFR-15).
 */
export default function InboxKeyboard() {
  const { t } = useTranslation("palette");
  const { t: tError } = useTranslation();
  const inbox = useInboxOffers();
  const cards = inbox.data;
  const [announcement, setAnnouncement] = useState("");
  const save = useSaveOffer();
  const dismiss = useDismissOffer();
  const undo = useUndoDismiss();
  const lastDismissed = useRef<string | null>(null);
  const restored = useRef(false);

  const titleOf = useCallback(
    (id: string): string =>
      cards.find((card) => card.posting_id === id)?.title || t("offer.untitled"),
    [cards, t],
  );

  /** The card as a sentence: again the Verdict as a word, never a colour. */
  const describe = useCallback(
    (id: string): string => {
      const card = cards.find((candidate) => candidate.posting_id === id);
      if (!card) return titleOf(id);
      const verdict = verdictId(card);
      return t("announce.offer", {
        title: card.title || t("offer.untitled"),
        verdict: verdict ? t(VERDICT_LABELS[verdict]) : t("announce.unscored"),
        fit: card.fit === null ? "—" : Math.round(card.fit),
        quality: card.quality === null ? "—" : Math.round(card.quality),
      });
    },
    [cards, t, titleOf],
  );

  // Announce a card once its data is here: a keypress can land before the list's
  // first page does, and "Untitled offer" would be a lie read aloud.
  const pending = useRef<string | null>(null);
  const announce = (id: string) => {
    if (!cards.some((card) => card.posting_id === id)) {
      pending.current = id;
      return;
    }
    pending.current = null;
    setAnnouncement(describe(id));
  };

  const failure = (error: unknown) =>
    isAppError(error) ? renderAppError(error, tError) : t("triage.error");

  const onSave = (id: string) => {
    save.mutate(id, {
      onSuccess: () => toast(t("triage.saved", { title: titleOf(id) })),
      onError: (error) => toast.error(failure(error)),
    });
  };

  const onUndo = (id: string) => {
    undo.mutate(id, {
      onSuccess: () => {
        lastDismissed.current = null;
        toast(t("triage.undone", { title: titleOf(id) }));
      },
      onError: (error) => toast.error(failure(error)),
    });
  };

  const onDismiss = (id: string) => {
    dismiss.mutate(
      { postingIds: [id], reason: null },
      {
        onSuccess: () => {
          lastDismissed.current = id;
          const title = titleOf(id);
          // The card is about to leave the list: move the cursor on before it goes.
          focusNeighbour(id);
          toast(t("triage.dismissed", { title }), {
            duration: UNDO_WINDOW_MS,
            action: { label: t("triage.undo"), onClick: () => onUndo(id) },
          });
        },
        onError: (error) => toast.error(failure(error)),
      },
    );
  };

  // The listener is bound once and reads the current handlers from this ref, so
  // it never races a render (a stale `cards` would announce the wrong Offer).
  const handlers = useRef({ save: onSave, dismiss: onDismiss, undo: onUndo, announce });
  handlers.current = { save: onSave, dismiss: onDismiss, undo: onUndo, announce };

  useEffect(() => {
    function onKeyDown(event: globalThis.KeyboardEvent) {
      const mod = event.metaKey || event.ctrlKey;
      const key = event.key.toLowerCase();

      if (mod && key === "z" && !event.shiftKey && !isEditable(event.target)) {
        const last = lastDismissed.current;
        if (!last) return;
        event.preventDefault();
        handlers.current.undo(last);
        return;
      }
      if (mod || event.altKey || event.shiftKey || isEditable(event.target)) return;

      if (key === "j" || key === "k") {
        event.preventDefault();
        const id = moveCursor(key === "j" ? 1 : -1);
        if (id) handlers.current.announce(id);
        return;
      }
      // `Enter` needs no handler: the cursor is the card's own link, and the
      // browser opens it. The same goes for `Space` on a focused control.
      if (key === "s" || key === "d" || key === "x") {
        const id = focusedPostingId();
        if (!id) return;
        event.preventDefault();
        if (key === "s") handlers.current.save(id);
        else handlers.current.dismiss(id);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  // Remember the card under the focus and say what it is (FR-68 + NFR-15).
  useEffect(() => {
    function onFocusIn(event: FocusEvent) {
      const id = postingIdOf(event.target as Element | null);
      if (!id) return;
      rememberCard(id);
      handlers.current.announce(id);
    }
    document.addEventListener("focusin", onFocusIn);
    return () => document.removeEventListener("focusin", onFocusIn);
  }, []);

  // Coming back from an Offer: put the cursor back on the card it left from,
  // once the list has drawn it again.
  useEffect(() => {
    if (restored.current || cards.length === 0) return;
    const id = rememberedCard();
    if (id && focusCard(id)) restored.current = true;
  }, [cards.length]);

  // A card announced before its data arrived says what it is now that it has.
  useEffect(() => {
    const id = pending.current;
    if (!id || cards.length === 0) return;
    pending.current = null;
    if (focusedPostingId() === id) setAnnouncement(describe(id));
  }, [cards.length, describe]);

  return (
    <>
      <p role="status" aria-live="polite" className="sr-only">
        {announcement}
      </p>
      <p className="text-xs text-fg-subtle">{t("triage.hint", { mod: modKey() })}</p>
    </>
  );
}
