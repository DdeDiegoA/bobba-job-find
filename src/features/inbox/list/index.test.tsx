import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OfferCardView } from "@/lib/ipc/bindings";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import InboxList from "./index";
import { inboxKey } from "./offers";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: { offersInbox: vi.fn() },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

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
    first_seen: "2026-10-10T00:00:00Z",
    posted_at: "2026-10-10T00:00:00Z",
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
    ...overrides,
  };
}

function renderList() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const result = render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <InboxList />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  return { ...result, client };
}

beforeEach(() => {
  vi.mocked(commands.offersInbox).mockReset();
});

describe("the sections (AC1)", () => {
  it("shows all six sections", async () => {
    vi.mocked(commands.offersInbox).mockReturnValue(ok([card()]));
    renderList();
    for (const name of [
      "Strong",
      "Good",
      "Moderate",
      "Weak / Poor",
      "Closing soon",
      "Excluded — review",
    ]) {
      expect(await screen.findByRole("button", { name: new RegExp(name) })).toBeTruthy();
    }
  });

  it("starts Weak/Poor collapsed and opens it on click", async () => {
    const low = card({ posting_id: "pst_low", title: "Junior Tester", verdict: "weak", rank: 35 });
    vi.mocked(commands.offersInbox).mockReturnValue(ok([low, card()]));
    renderList();

    const weak = await screen.findByRole("button", { name: /Weak \/ Poor/ });
    expect(weak.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText("Junior Tester")).toBeNull();

    fireEvent.click(weak);
    expect(weak.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("Junior Tester")).toBeTruthy();
  });

  it("keeps a vetoed offer out of the bands", async () => {
    const vetoed = card({
      posting_id: "pst_veto",
      title: "Site Reliability Eng",
      exclusion: {
        gate: "eligibility",
        reason: "requirement_not_met",
        quote: "Must be authorized to work in the US.",
      },
    });
    vi.mocked(commands.offersInbox).mockReturnValue(ok([vetoed]));
    renderList();

    expect(await screen.findByText(/Must be authorized to work in the US\./)).toBeTruthy();
    expect(screen.getByText(/VETO — work eligibility/)).toBeTruthy();
    // Its band says nothing: a VETO is not a ranked Offer (FR-54).
    expect(screen.getByRole("button", { name: /^Strong \(0\)/ })).toBeTruthy();
  });
});

describe("the Offer card (AC2)", () => {
  it("shows title, company, place, salary, both axes, age, Sources and badges", async () => {
    vi.mocked(commands.offersInbox).mockReturnValue(
      ok([
        card({
          sources: ["greenhouse", "rss"],
          badges: [{ id: "closing_soon", gate: "deadline" }],
          closing_soon: true,
        }),
      ]),
    );
    renderList();

    // Closing soon puts the same Offer in its band and in that section too
    // (§9.3), so the title is drawn twice on purpose.
    const [title] = await screen.findAllByText("Senior Data Engineer");
    expect(title).toBeTruthy();
    expect(screen.getAllByText(/Boba Labs · Remote, Spain · Remote/)[0]).toBeTruthy();
    expect(screen.getAllByText(/€55K per year/)[0]).toBeTruthy();
    expect(screen.getAllByText("81")[0]).toBeTruthy();
    expect(screen.getAllByText("71")[0]).toBeTruthy();
    // Verdict: text plus its icon, and the Rank beside it.
    expect(screen.getAllByText("Strong")[0]).toBeTruthy();
    expect(screen.getAllByText("78")[0]).toBeTruthy();
    expect(screen.getAllByText(/^Sources: greenhouse · rss$/)[0]).toBeTruthy();
    expect(screen.getAllByText("Closing soon")[0]).toBeTruthy();
    // The card shows the confidence in words (NFR-15: never colour alone).
    expect(screen.getAllByText(/confidence medium/)[0]).toBeTruthy();
    expect(screen.getAllByText(/\dd|today/)[0]).toBeTruthy();
  });

  it("shows a Low Confidence card as a dashed Lead", async () => {
    vi.mocked(commands.offersInbox).mockReturnValue(ok([card({ lead: true, confidence: "low" })]));
    renderList();

    // The Verdict filter also offers "Lead"; the chip is the one inside a card.
    const lead = await screen.findByText("Lead", { selector: "span" });
    expect(lead.parentElement?.className).toContain("border-dashed");
    expect(screen.getByText(/confidence low/)).toBeTruthy();
  });

  it("says an Offer is not scored yet instead of showing a zero", async () => {
    vi.mocked(commands.offersInbox).mockReturnValue(
      ok([
        card({
          posting_id: "pst_new",
          title: "Fresh Offer",
          fit: null,
          quality: null,
          rank: null,
          verdict: null,
          confidence: null,
        }),
      ]),
    );
    renderList();

    fireEvent.click(await screen.findByRole("button", { name: /Weak \/ Poor/ }));
    expect(await screen.findByText("Not scored yet")).toBeTruthy();
  });
});

describe("filters and sorts (AC3)", () => {
  const manual = card({
    posting_id: "pst_manual",
    title: "Manual Offer",
    sources: ["manual"],
    verdict: "good",
    rank: 66,
  });

  it("updates the list when a Source is picked", async () => {
    vi.mocked(commands.offersInbox).mockReturnValue(ok([card(), manual]));
    renderList();

    expect(await screen.findByText("Senior Data Engineer")).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Source"), { target: { value: "manual" } });

    expect(screen.queryByText("Senior Data Engineer")).toBeNull();
    expect(screen.getByText("Manual Offer")).toBeTruthy();
  });

  it("offers Clear filters when the filters empty the list", async () => {
    const noSalary = card({
      posting_id: "pst_nosal",
      title: "No Salary",
      amount_minor: null,
      currency: null,
      pay_period: null,
      verdict: "good",
    });
    vi.mocked(commands.offersInbox).mockReturnValue(ok([noSalary]));
    renderList();

    await screen.findByText("No Salary");
    fireEvent.click(screen.getByLabelText("Salary known"));

    expect(screen.getByText("No Offers for these filters.")).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: "Clear filters" }).at(-1) as HTMLElement);
    expect(await screen.findByText("No Salary")).toBeTruthy();
  });

  it("filters by Lead, the Verdict a Low Confidence card shows (FR-57)", async () => {
    const lead = card({ posting_id: "pst_lead", title: "Lead Offer", verdict: "good", lead: true });
    const plain = card({ posting_id: "pst_plain", title: "Plain Good", verdict: "good" });
    vi.mocked(commands.offersInbox).mockReturnValue(ok([lead, plain]));
    renderList();

    await screen.findByText("Plain Good");
    fireEvent.change(screen.getByLabelText("Verdict"), { target: { value: "lead" } });

    expect(screen.getByText("Lead Offer")).toBeTruthy();
    expect(screen.queryByText("Plain Good")).toBeNull();
  });

  it("sorts by Fit when asked, best first", async () => {
    const lowFit = card({ posting_id: "pst_lowfit", title: "Lower Fit", fit: 20, rank: 30 });
    vi.mocked(commands.offersInbox).mockReturnValue(ok([lowFit, card()]));
    renderList();

    await screen.findByText("Senior Data Engineer");
    fireEvent.change(screen.getByLabelText("Sort"), { target: { value: "fit" } });
    const titles = screen.getAllByRole("heading", { level: 3 }).map((node) => node.textContent);
    expect(titles.indexOf("Senior Data Engineer")).toBeLessThan(titles.indexOf("Lower Fit"));
  });
});

describe("loading and stale states (AC4)", () => {
  it("shows a skeleton while the first page loads", () => {
    vi.mocked(commands.offersInbox).mockReturnValue(new Promise(() => {}));
    const { container } = renderList();

    expect(screen.getByText("Loading your Offers…")).toBeTruthy();
    expect(container.querySelectorAll(".animate-pulse").length).toBeGreaterThan(0);
  });

  it("keeps the old list on screen and dims it while a re-run refreshes", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    client.setQueryData(inboxKey, [card()]);
    vi.mocked(commands.offersInbox).mockReturnValue(new Promise(() => {}));
    const { container } = render(
      <QueryClientProvider client={client}>
        <MemoryRouter>
          <InboxList />
        </MemoryRouter>
      </QueryClientProvider>,
    );

    // The previous list is still there…
    expect(await screen.findByText("Senior Data Engineer")).toBeTruthy();
    // …and the region is announced busy and drawn dimmed, never blank.
    const stale = container.querySelector("[aria-busy='true']");
    expect(stale).not.toBeNull();
    expect(stale?.className).toContain("opacity-60");
  });
});

describe("empty states", () => {
  it("shows the true empty state when there are no Offers at all", async () => {
    vi.mocked(commands.offersInbox).mockReturnValue(ok([]));
    renderList();

    expect(await screen.findByText("No Offers yet")).toBeTruthy();
    expect(screen.queryByText("No Offers for these filters.")).toBeNull();
  });
});
