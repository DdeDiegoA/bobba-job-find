import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OfferCardView } from "@/lib/ipc/bindings";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import TodayMatches from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: { offersInbox: vi.fn() },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

function card(overrides: Partial<OfferCardView> = {}): OfferCardView {
  return {
    posting_id: "pst_01",
    title: "Senior Data Engineer",
    company: "Boba Labs",
    locations: ["Remote"],
    work_mode: "remote",
    lang: "en",
    amount_minor: null,
    currency: null,
    pay_period: null,
    sources: ["greenhouse"],
    first_seen: "2026-10-10T00:00:00Z",
    posted_at: null,
    deadline: null,
    fit: 81,
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

function renderMatches() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <TodayMatches />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.offersInbox).mockReset();
});

describe("Today's matches (EXPERIENCE §9.2)", () => {
  it("counts the bands and links into the Inbox", async () => {
    vi.mocked(commands.offersInbox).mockReturnValue(
      ok([
        card({ posting_id: "a", verdict: "strong" }),
        card({ posting_id: "b", verdict: "strong" }),
        card({ posting_id: "c", verdict: "good", rank: 66 }),
      ]),
    );
    renderMatches();

    expect(await screen.findByText("2")).toBeTruthy();
    expect(screen.getByText("1")).toBeTruthy();
    expect(screen.getByText("Strong")).toBeTruthy();
    expect(screen.getByText("Good")).toBeTruthy();
    expect(screen.getByRole("link", { name: /Open Inbox/ })).toBeTruthy();
  });

  it("puts the closest Offers on top, by Rank", async () => {
    vi.mocked(commands.offersInbox).mockReturnValue(
      ok([
        card({ posting_id: "low", title: "Lower Ranked", rank: 55, verdict: "moderate" }),
        card({ posting_id: "high", title: "Best Match", rank: 88 }),
      ]),
    );
    renderMatches();

    expect(await screen.findByText("Best Match")).toBeTruthy();
    expect(screen.getByText("Lower Ranked")).toBeTruthy();
  });

  it("leaves vetoed and unscored Offers out of the counts", async () => {
    vi.mocked(commands.offersInbox).mockReturnValue(
      ok([
        card({ posting_id: "a" }),
        card({
          posting_id: "veto",
          exclusion: { gate: "eligibility", reason: "requirement_not_met", quote: "x" },
        }),
        card({ posting_id: "new", verdict: null, rank: null, fit: null, quality: null }),
      ]),
    );
    renderMatches();

    // One Strong, nothing else in the three bands on top of it.
    expect(await screen.findByText("1")).toBeTruthy();
  });

  it("shows a skeleton while the first page loads", () => {
    vi.mocked(commands.offersInbox).mockReturnValue(new Promise(() => {}));
    const { container } = renderMatches();

    expect(screen.getByText("Loading your matches…")).toBeTruthy();
    expect(container.querySelector(".animate-pulse")).not.toBeNull();
  });

  it("invites a first run when nothing is scored yet", async () => {
    vi.mocked(commands.offersInbox).mockReturnValue(ok([]));
    renderMatches();

    expect(await screen.findByText("No matches yet")).toBeTruthy();
  });
});
