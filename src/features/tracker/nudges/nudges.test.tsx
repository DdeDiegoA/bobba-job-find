import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  type ApplicationDetail,
  type BoardColumn,
  type BoardView,
  commands,
} from "@/lib/ipc/bindings";
import "@/lib/i18n";
import TrackerNudges from "./index";

vi.mock("sonner", () => ({ toast: vi.fn(() => "toast-1") }));

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    trackerBoard: vi.fn(),
    trackerApplicationDetail: vi.fn(),
    trackerTransitionStatus: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const daysAgo = (days: number) => new Date(Date.now() - days * 86_400_000).toISOString();

function boardView(): BoardView {
  const empty = (stage: string, status: string): BoardColumn => ({
    stage,
    status,
    terminal: ["hired", "rejected", "no_response", "withdrawn"].includes(stage),
    count: 0,
    cards: [],
  });
  const columns = [
    empty("saved", "saved"),
    empty("preparing", "drafted"),
    {
      stage: "applied",
      status: "applied",
      terminal: false,
      count: 1,
      cards: [
        {
          id: "app_01",
          status: "applied",
          stage: "applied",
          title: "Backend Engineer",
          company: "Boba Labs",
          next_action: null,
        },
      ],
    },
    empty("interviewing", "interview"),
    empty("offer", "offer"),
    empty("hired", "hired"),
    empty("rejected", "rejected"),
    empty("no_response", "no_response"),
    empty("withdrawn", "withdrawn"),
  ];
  return { columns };
}

function detailAppliedAt(days: number): ApplicationDetail {
  return {
    id: "app_01",
    posting_id: "pst_1",
    status: "applied",
    stage: "applied",
    title: "Backend Engineer",
    company: "Boba Labs",
    apply_url: null,
    lang: null,
    deadline: null,
    notes: null,
    next_action: null,
    package: [],
    timeline: [
      {
        id: "evt_1",
        from_status: "drafted",
        from_stage: "preparing",
        to_status: "applied",
        to_stage: "applied",
        source: "user",
        reason: null,
        created_at: daysAgo(days),
      },
    ],
    created_at: daysAgo(days + 5),
    updated_at: daysAgo(days),
  };
}

function renderNudges() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <TrackerNudges />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.trackerBoard).mockReset();
  vi.mocked(commands.trackerApplicationDetail).mockReset();
  vi.mocked(commands.trackerTransitionStatus).mockReset();
  vi.mocked(commands.trackerBoard).mockImplementation(() => ok(boardView()));
});

describe("follow-up nudges (AC2)", () => {
  it("stays quiet while the application is inside the first ten days", async () => {
    vi.mocked(commands.trackerApplicationDetail).mockImplementation(() => ok(detailAppliedAt(4)));
    renderNudges();
    await waitFor(() => expect(commands.trackerApplicationDetail).toHaveBeenCalledWith("app_01"));
    expect(screen.queryByText(/quiet for/)).toBeNull();
  });

  it("shows a check-in nudge after ten days without a reply", async () => {
    vi.mocked(commands.trackerApplicationDetail).mockImplementation(() => ok(detailAppliedAt(12)));
    renderNudges();
    expect(
      await screen.findByText(/Backend Engineer at Boba Labs has been quiet for 12 days/),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Mark as No Response" })).toBeNull();
  });

  it("suggests No Response at thirty days and only moves after the user confirms", async () => {
    vi.mocked(commands.trackerApplicationDetail).mockImplementation(() => ok(detailAppliedAt(31)));
    vi.mocked(commands.trackerTransitionStatus).mockImplementation(() => ok(detailAppliedAt(31)));
    renderNudges();

    fireEvent.click(await screen.findByRole("button", { name: "Mark as No Response" }));
    expect(await screen.findByText("Move Backend Engineer to No Response?")).toBeTruthy();
    // The suggestion itself never moves the card (AC2).
    expect(commands.trackerTransitionStatus).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Move to No Response" }));
    await waitFor(() =>
      expect(commands.trackerTransitionStatus).toHaveBeenCalledWith({
        application_id: "app_01",
        from_status: "applied",
        to_status: "no_response",
        source: "user",
        reason: "follow_up_nudge",
      }),
    );
  });
});
