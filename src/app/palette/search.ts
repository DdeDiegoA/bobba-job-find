import { useEffect, useState } from "react";
import { INBOX_PAGE } from "@/features/inbox/list/offers";
import { commands, type OfferCardView } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/**
 * The Offers the palette can search (S6.7).
 *
 * One page of the same core list the Inbox draws, fetched on the first open and
 * reused for a minute so typing never re-asks the core. It holds no query cache
 * of its own: the palette must render in the shell even when nothing else on
 * screen has, and a closed palette asks for nothing.
 */
const TTL_MS = 60_000;

let cached: { at: number; cards: OfferCardView[] } | null = null;

export function useOfferSearch(enabled: boolean): OfferCardView[] {
  const [cards, setCards] = useState<OfferCardView[]>([]);

  useEffect(() => {
    if (!enabled) return;
    if (cached && Date.now() - cached.at < TTL_MS) {
      setCards(cached.cards);
      return;
    }
    let alive = true;
    // `invoke` throws synchronously without a Tauri bridge (a browser preview);
    // the Promise chain keeps that from escaping the effect.
    void Promise.resolve()
      .then(() => commands.offersInbox(INBOX_PAGE, 0))
      .then(unwrap)
      .then((data) => {
        cached = { at: Date.now(), cards: data };
        if (alive) setCards(data);
      })
      .catch(() => {
        // No core to ask: the palette still navigates the pages.
      });
    return () => {
      alive = false;
    };
  }, [enabled]);

  return cards;
}
