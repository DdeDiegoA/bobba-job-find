import { describe, expect, it } from "vitest";
import type { OfferDetailView } from "@/lib/ipc/bindings";
import {
  ageDays,
  axisAnchor,
  factorGroups,
  payAmount,
  uiLanguage,
  verdictId,
  withoutRemoteContent,
} from "./detail";

/** An Offer detail, with the fields a test overrides. */
function detail(overrides: Partial<OfferDetailView> = {}): OfferDetailView {
  return {
    posting_id: "pst_01",
    title: "Senior Data Engineer",
    company: "Boba Labs",
    locations: ["Remote, Spain"],
    work_mode: "remote",
    lang: "en",
    amount_minor: 6_500_000,
    currency: "EUR",
    pay_period: "year",
    first_seen: "2026-10-14T00:00:00Z",
    posted_at: "2026-10-14T00:00:00Z",
    deadline: null,
    description_html: "<p>dbt and Airflow.</p>",
    requirements: ["dbt"],
    apply_url: "https://boards.example.com/boba/gh-42",
    apply_email: null,
    sources: ["greenhouse"],
    duplicates: [],
    channels: [{ kind: "manual", url: "https://boards.example.com/boba/gh-42" }],
    fit: 82.4,
    quality: 71,
    rank: 78,
    verdict: "strong",
    confidence: "medium",
    lead: false,
    gates: [],
    factors: [],
    narration: null,
    ...overrides,
  };
}

describe("the Offer detail helpers", () => {
  it("groups the factors by the axis whose anchor the DualScore scrolls to", () => {
    const view = detail({
      factors: [
        {
          factor: "f1_skills",
          axis: "fit",
          score: 88,
          weight: 0.3,
          effective_weight: 0.32,
          matched: ["dbt"],
          missing: [],
          evidence: [],
          confidence: "high",
        },
        {
          factor: "q1_compensation",
          axis: "quality",
          score: 92,
          weight: 0.25,
          effective_weight: 0.3,
          matched: ["above floor"],
          missing: [],
          evidence: [],
          confidence: "medium",
        },
      ],
    });
    const groups = factorGroups(view);
    expect(groups.map((group) => group.axis)).toEqual(["fit", "quality"]);
    expect(groups[0].anchor).toBe(axisAnchor("fit"));
    expect(groups[0].factors.map((factor) => factor.factor)).toEqual(["f1_skills"]);
    expect(groups[1].factors.map((factor) => factor.factor)).toEqual(["q1_compensation"]);
  });

  it("draws a low-confidence card as a Lead, not as its band", () => {
    expect(verdictId(detail({ lead: true }))).toBe("lead");
    expect(verdictId(detail())).toBe("strong");
    expect(verdictId(detail({ verdict: null }))).toBeNull();
  });

  it("formats pay from minor units, and nothing when the posting named none", () => {
    expect(payAmount(detail(), "en")).toContain("65,000");
    expect(payAmount(detail({ amount_minor: null }), "en")).toBeNull();
    expect(payAmount(detail({ currency: null }), "en")).toBeNull();
  });

  it("counts whole days and never goes negative", () => {
    const now = new Date("2026-10-16T12:00:00Z");
    expect(ageDays(detail(), now)).toBe(2);
    expect(ageDays(detail({ posted_at: null, first_seen: "2026-10-20T00:00:00Z" }), now)).toBe(0);
  });

  it("asks for the narration in the supported UI language", () => {
    expect(uiLanguage("es-ES")).toBe("es");
    expect(uiLanguage("en-US")).toBe("en");
    expect(uiLanguage(undefined)).toBe("en");
  });
});

describe("the remote-content strip (FR-65, AD-7)", () => {
  it("drops elements that could fetch something", () => {
    const html =
      '<p>dbt</p><img src="https://tracker.example/x.png"><iframe src="https://x"></iframe>';
    expect(withoutRemoteContent(html)).toBe("<p>dbt</p>");
  });

  it("drops a remote source attribute on a kept element", () => {
    const html = '<p style="color:red">dbt</p><div background="https://x/bg.png">x</div>';
    const cleaned = withoutRemoteContent(html);
    expect(cleaned).not.toContain("https://x/bg.png");
    expect(cleaned).toContain("dbt");
  });

  it("keeps the posting's own text intact", () => {
    const html = "<p>We use <strong>dbt</strong> and Airflow.</p>";
    expect(withoutRemoteContent(html)).toBe(html);
  });
});
