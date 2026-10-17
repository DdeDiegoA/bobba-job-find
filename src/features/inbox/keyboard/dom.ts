/**
 * The Inbox's card cursor (S6.7, FR-68).
 *
 * Cards are the `Link`s S6.4 already draws to `/offer/:id`, so the keyboard acts
 * on what the screen actually shows — no second list, no parallel state. One
 * entry per Offer: an Offer closing soon is drawn twice (§9.3) and must still be
 * visited once.
 */

/** The card links; the router writes `/offer/:id` as `#/offer/:id`. */
const CARD_LINK = 'a[href^="#/offer/"]';

/** Where the cards live: the shell's `<main id="main">`. */
function root(): ParentNode {
  return document.getElementById("main") ?? document;
}

/** The Offer id a card element (or a link inside it) stands for. */
export function postingIdOf(element: Element | null): string | null {
  const link = element?.closest(CARD_LINK);
  const href = link?.getAttribute("href") ?? "";
  const at = href.indexOf("/offer/");
  if (at === -1) return null;
  const raw = href.slice(at + "/offer/".length);
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}

/** One link per Offer, in the order the list draws them. */
export function uniqueCardLinks(
  scope: ParentNode = root(),
): { id: string; link: HTMLAnchorElement }[] {
  const entries: { id: string; link: HTMLAnchorElement }[] = [];
  const seen = new Set<string>();
  for (const link of scope.querySelectorAll<HTMLAnchorElement>(CARD_LINK)) {
    const id = postingIdOf(link);
    if (!id || seen.has(id)) continue;
    seen.add(id);
    entries.push({ id, link });
  }
  return entries;
}

/**
 * The Offer the user is on: the focused card link, or the link of the card
 * whose own buttons (Save, Dismiss) hold the focus.
 */
export function focusedPostingId(): string | null {
  const active = document.activeElement;
  const direct = postingIdOf(active);
  if (direct) return direct;
  const article = (active as HTMLElement | null)?.closest("article") ?? null;
  return article ? postingIdOf(article.querySelector(CARD_LINK)) : null;
}

/** Focus a card and keep it on screen; `false` when it is not drawn (yet). */
export function focusCard(id: string): boolean {
  const entry = uniqueCardLinks().find((candidate) => candidate.id === id);
  if (!entry) return false;
  entry.link.focus();
  // jsdom has no layout, so this is optional for the tests' sake.
  entry.link.scrollIntoView?.({ block: "nearest" });
  return true;
}

/**
 * Move the cursor by `delta` cards and return the Offer it landed on. With no
 * card focused, `J` enters at the first and `K` at the last.
 */
export function moveCursor(delta: 1 | -1): string | null {
  const links = uniqueCardLinks();
  if (links.length === 0) return null;
  const current = focusedPostingId();
  const index = current ? links.findIndex((entry) => entry.id === current) : -1;
  const next =
    index === -1
      ? delta > 0
        ? 0
        : links.length - 1
      : Math.min(links.length - 1, Math.max(0, index + delta));
  return focusCard(links[next].id) ? links[next].id : null;
}

/** Focus the card after `id`, else the one before it (after a dismissal). */
export function focusNeighbour(id: string): void {
  const links = uniqueCardLinks();
  const index = links.findIndex((entry) => entry.id === id);
  const target = index === -1 ? undefined : (links[index + 1] ?? links[index - 1]);
  target?.link.focus();
}

/**
 * The last card the user was on. It lives outside React so coming back from an
 * Offer restores the cursor, as EXPERIENCE §6 requires.
 */
let remembered: string | null = null;

export function rememberedCard(): string | null {
  return remembered;
}

export function rememberCard(id: string | null): void {
  remembered = id;
}
