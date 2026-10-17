import { describe, expect, it } from "vitest";
import type { OfferCardView } from "@/lib/ipc/bindings";
import {
  ageDays,
  bandCounts,
  cardsIn,
  type Filters,
  filterCards,
  filterOptions,
  hasFilters,
  NO_FILTERS,
  SECTIONS,
  salaryLabel,
  sortCards,
  VERDICT_FILTERS,
  verdictId,
} from "./offers";

/** A scored card, with the fields a test overrides. */
function card(overrides: Partial<OfferCardView> = {}): OfferCardView {
  return {
    posting_id: "pst_01",
    title: "Senior Data Engineer",
    company: "Boba Labs",
    locations: ["Remote, Spain"],
    work_mode: "remote",
    lang: "en",
    amount_minor: 5_500_000,
    currency: "EUR",
    pay_period: "year",
    sources: ["greenhouse"],
    first_seen: "2026-10-14T00:00:00Z",
    posted_at: "2026-10-14T00:00:00Z",
    deadline: null,
    fit: 81.3,
    quality: 71,
    rank: 78,
    verdict: "strong",
    confidence: "medium",
    lead: false,
    badges: [],
    exclusion: null,
    closing_soon: false,
    dismissed: false,
    overridden: false,
    ...overrides,
  };
}

describe("sections (FR-63)", () => {
  it("offers the six sections in the order the Inbox draws them", () => {
    expect(SECTIONS.map((section) => section.id)).toEqual([
      "strong",
      "good",
      "moderate",
      "weak_poor",
      "closing_soon",
      "excluded",
    ]);
  });

  it("starts Weak/Poor collapsed and the rest open", () => {
    const collapsed = SECTIONS.filter((section) => section.collapsed).map((section) => section.id);
    expect(collapsed).toEqual(["weak_poor"]);
  });

  it("puts each card in its band", () => {
    const cards = [
      card({ posting_id: "a", verdict: "strong" }),
      card({ posting_id: "b", verdict: "good" }),
      card({ posting_id: "c", verdict: "moderate" }),
      card({ posting_id: "d", verdict: "weak" }),
      card({ posting_id: "e", verdict: "poor" }),
    ];
    expect(cardsIn(cards, "strong").map((c) => c.posting_id)).toEqual(["a"]);
    expect(cardsIn(cards, "good").map((c) => c.posting_id)).toEqual(["b"]);
    expect(cardsIn(cards, "moderate").map((c) => c.posting_id)).toEqual(["c"]);
    expect(cardsIn(cards, "weak_poor").map((c) => c.posting_id)).toEqual(["d", "e"]);
  });

  // An Offer nobody scored yet is not a zero (AD-8): it joins the collapsed
  // group, with no band claimed.
  it("keeps an unscored offer in Weak/Poor without inventing a verdict", () => {
    const unscored = card({
      posting_id: "u",
      fit: null,
      quality: null,
      rank: null,
      verdict: null,
      confidence: null,
    });
    expect(cardsIn([unscored], "weak_poor").map((c) => c.posting_id)).toEqual(["u"]);
    expect(cardsIn([unscored], "strong")).toEqual([]);
    expect(verdictId(unscored)).toBeNull();
  });

  // §9.3: Closing soon is shown beside the band, not instead of it.
  it("lists a closing-soon offer in its band and in Closing soon", () => {
    const closing = card({ posting_id: "c", verdict: "good", closing_soon: true });
    expect(cardsIn([closing], "good").map((c) => c.posting_id)).toEqual(["c"]);
    expect(cardsIn([closing], "closing_soon").map((c) => c.posting_id)).toEqual(["c"]);
  });

  // FR-54: a vetoed Offer is Excluded, never ranked alongside the others.
  it("keeps a vetoed offer out of the bands and in Excluded", () => {
    const vetoed = card({
      posting_id: "x",
      verdict: "strong",
      exclusion: { gate: "eligibility", reason: "requirement_not_met", quote: "Must be…" },
    });
    expect(cardsIn([vetoed], "strong")).toEqual([]);
    expect(cardsIn([vetoed], "weak_poor")).toEqual([]);
    expect(cardsIn([vetoed], "excluded").map((c) => c.posting_id)).toEqual(["x"]);
  });

  it("counts the bands for Today, excluded apart from them", () => {
    const counts = bandCounts([
      card({ posting_id: "a", verdict: "strong" }),
      card({ posting_id: "b", verdict: "strong" }),
      card({ posting_id: "c", verdict: "good" }),
      card({ posting_id: "d", verdict: null, rank: null }),
      card({
        posting_id: "e",
        exclusion: { gate: "deadline", reason: "expired", quote: null },
      }),
    ]);
    expect(counts).toEqual({ strong: 2, good: 1, moderate: 0, weak_poor: 1, excluded: 1 });
  });
});

describe("verdict chip (FR-57/FR-64)", () => {
  it("shows a low-confidence card as a Lead, not as its band", () => {
    expect(verdictId(card({ verdict: "strong", lead: true }))).toBe("lead");
  });

  it("shows the band when the card could see enough", () => {
    expect(verdictId(card({ verdict: "moderate" }))).toBe("moderate");
  });
});

describe("filters (FR-63)", () => {
  const cards = [
    card({
      posting_id: "a",
      sources: ["greenhouse", "rss"],
      work_mode: "remote",
      lang: "en",
      verdict: "strong",
      amount_minor: 5_000_000,
    }),
    card({
      posting_id: "b",
      sources: ["manual"],
      work_mode: "onsite",
      lang: "es",
      verdict: "good",
      amount_minor: null,
      currency: null,
      pay_period: null,
    }),
  ];

  it("filters by Source, work mode, language and Verdict", () => {
    const ids = (value: Partial<Filters>) =>
      filterCards(cards, { ...NO_FILTERS, ...value }).map((c) => c.posting_id);
    expect(ids({ source: "rss" })).toEqual(["a"]);
    expect(ids({ workMode: "onsite" })).toEqual(["b"]);
    expect(ids({ lang: "es" })).toEqual(["b"]);
    expect(ids({ verdict: "strong" })).toEqual(["a"]);
    expect(ids({ source: "manual", salaryKnown: true })).toEqual([]);
  });

  // FR-57/AC3: a Lead is the Verdict the card shows, so it is a filter choice of
  // its own — and a Lead is not also reachable under its band.
  it("filters by Lead, and the band filter keeps only cards shown as that band", () => {
    const lead = card({ posting_id: "l", verdict: "good", lead: true });
    const plain = card({ posting_id: "p", verdict: "good", lead: false });
    expect(
      filterCards([lead, plain], { ...NO_FILTERS, verdict: "lead" }).map((c) => c.posting_id),
    ).toEqual(["l"]);
    expect(
      filterCards([lead, plain], { ...NO_FILTERS, verdict: "good" }).map((c) => c.posting_id),
    ).toEqual(["p"]);
    expect(VERDICT_FILTERS).toContain("lead");
  });

  it("keeps only Offers whose salary is known", () => {
    expect(
      filterCards(cards, { ...NO_FILTERS, salaryKnown: true }).map((c) => c.posting_id),
    ).toEqual(["a"]);
  });

  it("knows when a filter is on — the filtered-empty state offers Clear only then", () => {
    expect(hasFilters(NO_FILTERS)).toBe(false);
    expect(hasFilters({ ...NO_FILTERS, salaryKnown: true })).toBe(true);
    expect(hasFilters({ ...NO_FILTERS, source: "rss" })).toBe(true);
  });

  it("offers each Source, work mode and language once, sorted", () => {
    expect(filterOptions(cards)).toEqual({
      sources: ["greenhouse", "manual", "rss"],
      workModes: ["onsite", "remote"],
      langs: ["en", "es"],
    });
  });
});

describe("sorts (FR-63)", () => {
  const cards = [
    card({ posting_id: "a", rank: 60, fit: 90, quality: 40, posted_at: "2026-10-01T00:00:00Z" }),
    card({ posting_id: "b", rank: 80, fit: 70, quality: 88, posted_at: "2026-10-10T00:00:00Z" }),
    card({
      posting_id: "c",
      rank: null,
      fit: null,
      quality: null,
      verdict: null,
      posted_at: null,
      first_seen: "2026-10-05T00:00:00Z",
    }),
  ];

  it("sorts by Rank, Fit and Quality, highest first", () => {
    expect(sortCards(cards, "rank").map((c) => c.posting_id)).toEqual(["b", "a", "c"]);
    expect(sortCards(cards, "fit").map((c) => c.posting_id)).toEqual(["a", "b", "c"]);
    expect(sortCards(cards, "quality").map((c) => c.posting_id)).toEqual(["b", "a", "c"]);
  });

  // A card with no number for the chosen sort goes last, never as a zero.
  it("puts cards without that number last", () => {
    expect(sortCards(cards, "fit").at(-1)?.posting_id).toBe("c");
  });

  it("sorts by date, newest first, falling back to first seen", () => {
    expect(sortCards(cards, "date").map((c) => c.posting_id)).toEqual(["b", "c", "a"]);
  });
});

describe("what the card reads (FR-64)", () => {
  it("ages a card from when it was posted", () => {
    const posted = card({ posted_at: "2026-10-10T00:00:00Z" });
    expect(ageDays(posted, new Date("2026-10-16T12:00:00Z"))).toBe(6);
  });

  it("falls back to first seen when the posting named no date", () => {
    const seen = card({ posted_at: null, first_seen: "2026-10-15T00:00:00Z" });
    expect(ageDays(seen, new Date("2026-10-16T00:00:00Z"))).toBe(1);
  });

  it("never reports a negative age", () => {
    const future = card({ posted_at: "2026-10-20T00:00:00Z" });
    expect(ageDays(future, new Date("2026-10-16T00:00:00Z"))).toBe(0);
  });

  it("formats pay in the card's currency, or nothing when unknown", () => {
    expect(salaryLabel(card(), "en")).toBeTruthy();
    expect(salaryLabel(card({ amount_minor: null, currency: null }), "en")).toBeNull();
  });

  it("adds a converted amount only when FX produced one, labelled with its rate date", () => {
    const conversion = { amount_minor: 5_000_000, currency: "USD", as_of: "2026-10-16" };
    expect(salaryLabel(card(), "en-US")).not.toContain("≈");
    const label = salaryLabel(card(), "en-US", conversion);
    expect(label).toContain("≈");
    expect(label).toContain("Oct 16, 2026");
  });
});

describe("the Inbox decisions (S6.6)", () => {
  const veto = {
    gate: "eligibility",
    reason: "requirement_not_met",
    quote: "Must be authorized to work in the US.",
  };

  // AC1: a dismissed Offer leaves the sections until Undo.
  it("draws no dismissed card in any section", () => {
    const cards = [card({ posting_id: "gone", dismissed: true }), card({ posting_id: "live" })];
    for (const section of SECTIONS) {
      expect(cardsIn(cards, section.id).map((c) => c.posting_id)).not.toContain("gone");
    }
    expect(bandCounts(cards).strong).toBe(1);
  });

  // AC2: an overridden VETO is shown in its band, not under Excluded.
  it("shows an overridden veto in its band instead of Excluded", () => {
    const cards = [card({ posting_id: "over", exclusion: veto, overridden: true })];
    expect(cardsIn(cards, "excluded")).toHaveLength(0);
    expect(cardsIn(cards, "strong").map((c) => c.posting_id)).toEqual(["over"]);
    expect(bandCounts(cards).excluded).toBe(0);
  });

  it("still excludes a veto nobody overrode", () => {
    const cards = [card({ posting_id: "veto", exclusion: veto })];
    expect(cardsIn(cards, "excluded").map((c) => c.posting_id)).toEqual(["veto"]);
    expect(cardsIn(cards, "strong")).toHaveLength(0);
  });
});
