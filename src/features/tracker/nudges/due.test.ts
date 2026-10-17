import { describe, expect, it } from "vitest";
import type { ApplicationDetail, StatusEventView } from "@/lib/ipc/bindings";
import { appliedAt, CHECK_IN_DAYS, NO_RESPONSE_DAYS, nudgeFor, nudgesFor } from "./due";

const NOW = new Date("2026-10-17T12:00:00Z");

const daysAgo = (days: number) => new Date(NOW.getTime() - days * 86_400_000).toISOString();

function event(to_status: string, at: string): StatusEventView {
  return {
    id: `evt_${to_status}_${at}`,
    from_status: to_status === "applied" ? "drafted" : "applied",
    from_stage: to_status === "applied" ? "preparing" : "applied",
    to_status,
    to_stage: to_status === "applied" ? "applied" : "no_response",
    source: "user",
    reason: null,
    created_at: at,
  };
}

function detail(
  status: string,
  timeline: StatusEventView[],
  overrides: Partial<ApplicationDetail> = {},
): ApplicationDetail {
  return {
    id: "app_01",
    posting_id: "pst_1",
    status,
    stage: status,
    title: "Backend Engineer",
    company: "Boba Labs",
    apply_url: null,
    lang: null,
    deadline: null,
    notes: null,
    next_action: null,
    package: [],
    timeline,
    created_at: daysAgo(60),
    updated_at: daysAgo(1),
    ...overrides,
  };
}

describe("appliedAt", () => {
  it("returns the moment the application last entered Applied", () => {
    const card = detail("applied", [event("applied", daysAgo(40)), event("applied", daysAgo(12))]);
    expect(appliedAt(card)).toBe(daysAgo(12));
  });

  it("returns null when the timeline never entered Applied", () => {
    expect(appliedAt(detail("drafted", []))).toBeNull();
  });
});

describe("nudgeFor", () => {
  it("silently waits inside the first days", () => {
    const card = detail("applied", [event("applied", daysAgo(CHECK_IN_DAYS - 1))]);
    expect(nudgeFor(card, NOW)).toBeNull();
  });

  it("nudges a check-in once ten days pass without a reply", () => {
    const card = detail("applied", [event("applied", daysAgo(CHECK_IN_DAYS))]);
    expect(nudgeFor(card, NOW)).toEqual({
      applicationId: "app_01",
      company: "Boba Labs",
      title: "Backend Engineer",
      kind: "check_in",
      days: CHECK_IN_DAYS,
    });
  });

  it("suggests No Response once thirty days pass", () => {
    const card = detail("applied", [event("applied", daysAgo(NO_RESPONSE_DAYS + 2))]);
    expect(nudgeFor(card, NOW)?.kind).toBe("no_response");
    expect(nudgeFor(card, NOW)?.days).toBe(NO_RESPONSE_DAYS + 2);
  });

  it("stops nudging once the card left Applied", () => {
    const card = detail("interview", [event("applied", daysAgo(40))]);
    expect(nudgeFor(card, NOW)).toBeNull();
  });

  it("skips a timeline date it cannot read", () => {
    const card = detail("applied", [
      { ...event("applied", "2026-10-01T00:00:00Z"), created_at: "not a date" },
    ]);
    expect(nudgeFor(card, NOW)).toBeNull();
  });
});

describe("nudgesFor", () => {
  it("lists the No Response suggestions before the check-ins", () => {
    const checkIn = detail("applied", [event("applied", daysAgo(12))]);
    const stale = detail("applied", [event("applied", daysAgo(31))], {
      id: "app_02",
      title: "Data Engineer",
    });
    expect(nudgesFor([checkIn, stale], NOW).map((nudge) => nudge.applicationId)).toEqual([
      "app_02",
      "app_01",
    ]);
  });
});
