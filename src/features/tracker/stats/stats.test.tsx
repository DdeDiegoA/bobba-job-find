import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type FunnelStats } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import TrackerStats from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: { statsFunnel: vi.fn() },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

function stats(over: Partial<FunnelStats> = {}): FunnelStats {
  return {
    applications: 3,
    applied: 2,
    replies: 1,
    interviews: 1,
    reply_rate: 0.5,
    interview_rate: 0.5,
    by_stage: [
      { stage: "saved", count: 1 },
      { stage: "applied", count: 1 },
      { stage: "interviewing", count: 1 },
    ],
    by_source: [
      {
        key: "src_a",
        label: "Greenhouse",
        applications: 2,
        replies: 1,
        interviews: 1,
        reply_rate: 0.5,
        interview_rate: 0.5,
      },
    ],
    by_verdict: [
      {
        key: "strong",
        label: null,
        applications: 2,
        replies: 1,
        interviews: 1,
        reply_rate: 0.5,
        interview_rate: 0.5,
      },
    ],
    by_language: [
      {
        key: "en",
        label: null,
        applications: 2,
        replies: 1,
        interviews: 1,
        reply_rate: 0.5,
        interview_rate: 0.5,
      },
    ],
    ...over,
  };
}

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <TrackerStats />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(commands.statsFunnel).mockImplementation(() => ok(stats()));
});

describe("Tracker → Stats (FR-97, SM-4/SM-5)", () => {
  it("shows the funnel and the three breakdowns, computed locally (AC2)", async () => {
    renderPage();

    // Wait for the data, then read the four summary figures: total, applied,
    // reply rate, interview rate.
    expect(await screen.findByText("By Source")).toBeTruthy();
    expect(screen.getAllByRole("definition").map((node) => node.textContent)).toEqual([
      "3",
      "2",
      "50%",
      "50%",
    ]);
    // Applications by Stage keeps every column the core returned.
    expect(screen.getByText("Saved")).toBeTruthy();
    expect(screen.getByText("Interviewing")).toBeTruthy();
    // The Source row shows the name; the Verdict and language rows are labelled.
    expect(screen.getByText("Greenhouse")).toBeTruthy();
    expect(screen.getByText("Strong")).toBeTruthy();
    expect(screen.getByText("EN")).toBeTruthy();
  });

  it("renders the empty state before the first Application", async () => {
    vi.mocked(commands.statsFunnel).mockImplementation(() =>
      ok(
        stats({
          applications: 0,
          applied: 0,
          replies: 0,
          interviews: 0,
          reply_rate: 0,
          interview_rate: 0,
          by_source: [],
          by_verdict: [],
          by_language: [],
        }),
      ),
    );
    renderPage();
    expect(await screen.findByText("No applications yet.")).toBeTruthy();
  });
});
