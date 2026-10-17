import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import "@/lib/i18n";
import { commands, type OfferDetailView } from "@/lib/ipc/bindings";
import OfferDetail from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: { offerDetail: vi.fn() },
}));
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn(() => Promise.resolve()),
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

/** An Offer with Gates, factors, a narration and two channels. */
function view(overrides: Partial<OfferDetailView> = {}): OfferDetailView {
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
    description_html: "<p>We use <strong>dbt</strong> and Airflow.</p>",
    requirements: ["dbt"],
    apply_url: "https://boards.example.com/boba/gh-42",
    apply_email: "jobs@boba.example",
    sources: ["greenhouse", "rss"],
    duplicates: [
      {
        source: "rss",
        external_id: "rss-7",
        url: "https://feed.example.com/jobs/7",
        fetched_at: "2026-10-14T00:00:00Z",
      },
    ],
    channels: [
      { kind: "email_apply", address: "jobs@boba.example" },
      { kind: "manual", url: "https://boards.example.com/boba/gh-42" },
    ],
    fit: 82.4,
    quality: 71,
    rank: 78,
    verdict: "strong",
    confidence: "medium",
    lead: false,
    gates: [
      {
        gate: "language",
        verdict: "flag",
        reason: "unverified",
        confidence: "medium",
        quote: "English B2 required",
        field: null,
      },
    ],
    factors: [
      {
        factor: "f1_skills",
        axis: "fit",
        score: 88,
        weight: 0.3,
        effective_weight: 0.32,
        matched: ["dbt", "Airflow"],
        missing: ["Kubernetes"],
        evidence: [{ kind: "quote", text: "at least five years of dbt" }],
        confidence: "high",
      },
      {
        factor: "q1_compensation",
        axis: "quality",
        score: 92,
        weight: 0.25,
        effective_weight: 0.3,
        matched: ["above your floor"],
        missing: [],
        evidence: [],
        confidence: "medium",
      },
    ],
    narration: {
      text: "You match most of the ask, with dbt and Airflow standing out. Kubernetes is the one skill missing.",
      lang: "en",
      untrusted_origin: true,
    },
    ...overrides,
  };
}

function renderPage(postingId = "pst_01") {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[`/offer/${postingId}`]}>
        <Routes>
          <Route path="/offer/:postingId" element={<OfferDetail />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("the Offer detail screen (FR-65)", () => {
  beforeEach(() => {
    vi.mocked(commands.offerDetail).mockReset();
  });

  it("asks for the detail in the UI language", async () => {
    vi.mocked(commands.offerDetail).mockReturnValue(ok(view()));
    renderPage();
    await screen.findByText("Senior Data Engineer");
    expect(commands.offerDetail).toHaveBeenCalledWith("pst_01", expect.stringMatching(/^en/));
  });

  it("shows the Gates with their quotes, the factors and the channels", async () => {
    vi.mocked(commands.offerDetail).mockReturnValue(ok(view()));
    renderPage();

    // AC1: Gates first, with the posting's own quote as text.
    expect(await screen.findByText("Language")).toBeTruthy();
    expect(screen.getByText(/English B2 required/)).toBeTruthy();

    // AC1: the breakdown — matched and missing skills — and the evidence.
    expect(screen.getByText("Kubernetes")).toBeTruthy();
    expect(screen.getAllByText("dbt").length).toBeGreaterThan(0);
    expect(screen.getByText(/at least five years of dbt/)).toBeTruthy();

    // AC2: the Explanation is rendered as plain text.
    const narration = screen.getByText(/You match most of the ask/);
    expect(narration.tagName).toBe("P");
    expect(narration.textContent).not.toContain("<");

    // AC1: Sources, the merged copy, and the Apply Channels.
    expect(screen.getByText("greenhouse · rss")).toBeTruthy();
    expect(screen.getByText(/rss · rss-7/)).toBeTruthy();
    expect(screen.getByText(/Email apply · jobs@boba.example/)).toBeTruthy();
    expect(screen.getByRole("button", { name: /Open original/ })).toBeTruthy();
  });

  it("renders the sanitised posting without loading remote content", async () => {
    vi.mocked(commands.offerDetail).mockReturnValue(
      ok(
        view({
          description_html:
            '<p>We use <strong>dbt</strong> and Airflow.</p><img src="https://tracker.example/x.png">',
        }),
      ),
    );
    const { container } = renderPage();
    await screen.findByText("Senior Data Engineer");
    expect(container.innerHTML).toContain("<strong>dbt</strong>");
    expect(container.querySelector("img")).toBeNull();
  });

  it("scrolls to the factor group when an axis is clicked (AC3)", async () => {
    vi.mocked(commands.offerDetail).mockReturnValue(ok(view()));
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    renderPage();
    await screen.findByText("Senior Data Engineer");

    fireEvent.click(screen.getByRole("button", { name: "See the Fit factors" }));
    const group = document.getElementById("offer-factor-fit");
    expect(group).not.toBeNull();
    expect(scrollIntoView).toHaveBeenCalledTimes(1);
    expect(scrollIntoView.mock.instances[0]).toBe(group);

    fireEvent.click(screen.getByRole("button", { name: "See the Quality factors" }));
    expect(scrollIntoView).toHaveBeenCalledTimes(2);
    expect(scrollIntoView.mock.instances[1]).toBe(document.getElementById("offer-factor-quality"));
  });

  it("says so when no narration has been written", async () => {
    vi.mocked(commands.offerDetail).mockReturnValue(ok(view({ narration: null })));
    renderPage();
    await screen.findByText("Senior Data Engineer");
    expect(screen.getByText(/has not written an explanation/)).toBeTruthy();
  });

  it("shows the typed error with a retry", async () => {
    vi.mocked(commands.offerDetail).mockReturnValue(
      Promise.resolve({
        status: "error" as const,
        error: { code: "internal", message_key: "errors.offers.not_found", details: null },
      }),
    );
    renderPage();
    expect(await screen.findByRole("alert")).toBeTruthy();
    await waitFor(() => expect(screen.getByText(/could not open this Offer/i)).toBeTruthy());
  });
});
