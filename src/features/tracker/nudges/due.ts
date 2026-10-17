import type { ApplicationDetail } from "@/lib/ipc/bindings";

/**
 * Follow-up nudges (S9.5, FR-95). Pure: the UI hands in the card and now, and
 * gets back the nudge it should show, if any.
 *
 * An Application still in Applied ten days after it was sent has gone quiet;
 * at thirty days Boba suggests moving it to No Response — a suggestion only,
 * the move still waits for the user's confirmation (AC2).
 */
export const CHECK_IN_DAYS = 10;
export const NO_RESPONSE_DAYS = 30;

const DAY_MS = 86_400_000;

export type NudgeKind = "check_in" | "no_response";

export type Nudge = {
  applicationId: string;
  company: string;
  title: string;
  kind: NudgeKind;
  days: number;
};

/**
 * When the application last entered Applied, or null when it never did.
 *
 * Compared as instants, not as strings: the core's RFC 3339 writer drops
 * trailing zeros in fractional seconds, so `…09:00:00.5Z` sorts before
 * `…09:00:00Z` in lexicographic order although it is later (S9.5 F1).
 */
export function appliedAt(detail: ApplicationDetail): string | null {
  let latest: string | null = null;
  let latestAt = Number.NEGATIVE_INFINITY;
  for (const event of detail.timeline) {
    if (event.to_status !== "applied") continue;
    const at = Date.parse(event.created_at);
    if (Number.isNaN(at)) continue;
    if (at > latestAt) {
      latestAt = at;
      latest = event.created_at;
    }
  }
  return latest;
}

/** Days between an RFC 3339 timestamp and `now`, or null when unreadable. */
function daysSince(at: string, now: Date): number | null {
  const then = Date.parse(at);
  if (Number.isNaN(then)) return null;
  return Math.max(0, Math.floor((now.getTime() - then) / DAY_MS));
}

/** The nudge one card deserves right now, if any. */
export function nudgeFor(detail: ApplicationDetail, now: Date): Nudge | null {
  if (detail.status !== "applied") return null;
  const at = appliedAt(detail);
  if (at === null) return null;
  const days = daysSince(at, now);
  if (days === null || days < CHECK_IN_DAYS) return null;
  return {
    applicationId: detail.id,
    company: detail.company,
    title: detail.title,
    kind: days >= NO_RESPONSE_DAYS ? "no_response" : "check_in",
    days,
  };
}

/** Every nudge to show, No Response suggestions first. */
export function nudgesFor(details: ApplicationDetail[], now: Date): Nudge[] {
  const nudges = details
    .map((detail) => nudgeFor(detail, now))
    .filter((nudge): nudge is Nudge => nudge !== null);
  return nudges.sort((a, b) =>
    a.kind === b.kind ? b.days - a.days : a.kind === "no_response" ? -1 : 1,
  );
}
