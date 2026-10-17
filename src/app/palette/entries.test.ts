import { describe, expect, it } from "vitest";
import type { OfferCardView } from "@/lib/ipc/bindings";
import { offerEntries, type PaletteEntry, pageEntries, rank } from "./entries";

const entry = (overrides: Partial<PaletteEntry> = {}): PaletteEntry => ({
  id: "page:/x",
  kind: "page",
  label: "Inbox",
  detail: null,
  to: "/x",
  ...overrides,
});

const card = (overrides: Partial<OfferCardView> = {}): OfferCardView => ({
  posting_id: "pst_01",
  title: "Senior Data Engineer",
  company: "Boba Labs",
  locations: [],
  work_mode: "remote",
  lang: "en",
  amount_minor: null,
  currency: null,
  pay_period: null,
  sources: [],
  first_seen: "2026-10-10T00:00:00Z",
  posted_at: null,
  deadline: null,
  fit: null,
  quality: null,
  rank: null,
  verdict: null,
  confidence: null,
  lead: false,
  badges: [],
  exclusion: null,
  closing_soon: false,
  dismissed: false,
  overridden: false,
  ...overrides,
});

describe("the palette's rows", () => {
  it("puts every page in sidebar order", () => {
    const rows = pageEntries((key) => key.replace("shell:nav.", ""));
    expect(rows.map((row) => row.to)).toEqual([
      "/",
      "/inbox",
      "/studio",
      "/tracker",
      "/profile",
      "/connections",
      "/settings",
    ]);
    expect(rows[0].label).toBe("today");
  });

  it("turns Offers into rows and leaves dismissed ones out", () => {
    const rows = offerEntries(
      [card(), card({ posting_id: "pst_gone", dismissed: true })],
      "Untitled offer",
    );
    expect(rows).toHaveLength(1);
    expect(rows[0].to).toBe("/offer/pst_01");
    expect(rows[0].detail).toBe("Boba Labs");
    expect(offerEntries([card({ title: "" })], "Untitled offer")[0].label).toBe("Untitled offer");
  });
});

describe("matching", () => {
  it("keeps every row for an empty query", () => {
    const rows = [entry(), entry({ id: "page:/y", label: "Studio" })];
    expect(rank(rows, "  ").map((row) => row.label)).toEqual(["Inbox", "Studio"]);
  });

  it("is case- and accent-insensitive (FR-114)", () => {
    const rows = [entry({ label: "Español" })];
    expect(rank(rows, "espanol")).toHaveLength(1);
    expect(rank(rows, "ESPAÑOL")).toHaveLength(1);
  });

  it("ranks a label match above a company match, and a prefix first", () => {
    const rows = [
      entry({ id: "offer:1", kind: "offer", label: "Backend Engineer", detail: "Tracker Inc" }),
      entry({ id: "page:/tracker", label: "Tracker" }),
      entry({ id: "page:/other", label: "Data Tracker" }),
    ];
    expect(rank(rows, "tracker").map((row) => row.id)).toEqual([
      "page:/tracker",
      "page:/other",
      "offer:1",
    ]);
  });

  it("drops rows that do not match at all", () => {
    expect(rank([entry()], "zzz")).toEqual([]);
  });
});
