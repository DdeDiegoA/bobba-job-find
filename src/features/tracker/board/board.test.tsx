import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { toast } from "sonner";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { type ApplicationDetail, type BoardColumn, commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import { Board } from "./board";

vi.mock("sonner", () => ({ toast: vi.fn(() => "toast-1"), Toaster: () => null }));

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    trackerBoard: vi.fn(),
    trackerApplicationDetail: vi.fn(),
    trackerTransitionStatus: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const card = (id: string, title: string, status: string, stage: string) => ({
  id,
  status,
  stage,
  title,
  company: "Boba Labs",
  next_action: null,
});

/** The nine columns the core sends; only the ones a test cares about carry cards. */
function board(...withCards: BoardColumn[]): { columns: BoardColumn[] } {
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
    empty("applied", "applied"),
    empty("interviewing", "interview"),
    empty("offer", "offer"),
    empty("hired", "hired"),
    empty("rejected", "rejected"),
    empty("no_response", "no_response"),
    empty("withdrawn", "withdrawn"),
  ];
  for (const replacement of withCards) {
    columns[columns.findIndex((column) => column.stage === replacement.stage)] = replacement;
  }
  return { columns };
}

const detail: ApplicationDetail = {
  id: "app_01",
  posting_id: "pst_1",
  status: "saved",
  stage: "saved",
  title: "Backend Engineer",
  company: "Boba Labs",
  apply_url: "https://jobs.example.com/abc-1",
  lang: "en",
  deadline: null,
  notes: "referred by Ana",
  next_action: "follow up on Friday",
  package: [
    {
      document_id: "doc_1",
      kind: "cv",
      lang: "en",
      version: 2,
      verified: true,
    },
  ],
  timeline: [
    {
      id: "evt_1",
      from_status: null,
      from_stage: null,
      to_status: "saved",
      to_stage: "saved",
      source: "user",
      reason: null,
      created_at: "2026-10-16T09:00:00Z",
    },
    {
      id: "evt_2",
      from_status: "saved",
      from_stage: "saved",
      to_status: "interview",
      to_stage: "interviewing",
      source: "status_proposal",
      reason: null,
      created_at: "2026-10-17T09:00:00Z",
    },
  ],
  created_at: "2026-10-16T09:00:00Z",
  updated_at: "2026-10-16T09:00:00Z",
};

function renderBoard() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Board />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.trackerBoard).mockReset();
  vi.mocked(commands.trackerApplicationDetail).mockReset();
  vi.mocked(commands.trackerTransitionStatus).mockReset();
  vi.mocked(toast).mockClear();
  vi.mocked(commands.trackerBoard).mockImplementation(() =>
    ok(
      board({
        stage: "saved",
        status: "saved",
        terminal: false,
        count: 1,
        cards: [card("app_01", "Backend Engineer", "saved", "saved")],
      }),
    ),
  );
  vi.mocked(commands.trackerApplicationDetail).mockImplementation(() => ok(detail));
  vi.mocked(commands.trackerTransitionStatus).mockImplementation(() => ok(detail));
});

describe("Tracker board (AC1)", () => {
  it("renders the nine stage columns with their counts", async () => {
    renderBoard();
    for (const label of [
      "Saved",
      "Preparing",
      "Applied",
      "Interviewing",
      "Offer",
      "Hired",
      "Rejected",
      "No Response",
      "Withdrawn",
    ]) {
      expect(await screen.findByRole("heading", { name: new RegExp(label) })).toBeTruthy();
    }
    expect(screen.getByText("Backend Engineer")).toBeTruthy();
    expect(screen.getByText("Boba Labs")).toBeTruthy();
  });

  it("collapses a terminal column and keeps the others open", async () => {
    renderBoard();
    const collapse = await screen.findByRole("button", { name: "Hide Hired" });
    fireEvent.click(collapse);
    expect(screen.getByRole("button", { name: "Show Hired" })).toBeTruthy();
    // Only the terminal columns offer collapsing (FR-93).
    expect(screen.queryByRole("button", { name: "Hide Saved" })).toBeNull();
  });
});

describe("Tracker moves (AC2/AC4)", () => {
  it("logs a move and offers Undo, which moves the card back", async () => {
    renderBoard();
    const cardEl = await screen.findByLabelText("Open Backend Engineer");
    fireEvent.keyDown(cardEl, { key: "ArrowRight", metaKey: true });

    await waitFor(() =>
      expect(commands.trackerTransitionStatus).toHaveBeenCalledWith({
        application_id: "app_01",
        from_status: "saved",
        to_status: "drafted",
        source: "user",
        reason: null,
      }),
    );

    const [message, options] = vi.mocked(toast).mock.calls[0] as unknown as [
      string,
      { action: { onClick: () => void } },
    ];
    expect(message).toBe("Moved to Preparing");
    options.action.onClick();

    await waitFor(() => expect(commands.trackerTransitionStatus).toHaveBeenCalledTimes(2));
    expect(vi.mocked(commands.trackerTransitionStatus).mock.calls[1][0]).toMatchObject({
      from_status: "drafted",
      to_status: "saved",
    });
  });

  it("asks for a light confirmation before marking a card Applied", async () => {
    vi.mocked(commands.trackerBoard).mockImplementation(() =>
      ok(
        board({
          stage: "interviewing",
          status: "interview",
          terminal: false,
          count: 1,
          cards: [card("app_02", "Data Engineer", "interview", "interviewing")],
        }),
      ),
    );
    renderBoard();
    const cardEl = await screen.findByLabelText("Open Data Engineer");
    fireEvent.keyDown(cardEl, { key: "ArrowLeft", metaKey: true });

    expect(await screen.findByText("Mark Data Engineer as Applied?")).toBeTruthy();
    expect(commands.trackerTransitionStatus).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Move to Applied" }));
    await waitFor(() =>
      expect(commands.trackerTransitionStatus).toHaveBeenCalledWith({
        application_id: "app_02",
        from_status: "interview",
        to_status: "applied",
        source: "user",
        reason: null,
      }),
    );
  });

  it("re-reads the board when a move lost the race, so the copy is true", async () => {
    vi.mocked(commands.trackerTransitionStatus).mockImplementation(() =>
      Promise.reject({ code: "conflict", message_key: "errors.tracker.stale" }),
    );
    renderBoard();
    const cardEl = await screen.findByLabelText("Open Backend Engineer");
    expect(vi.mocked(commands.trackerBoard).mock.calls.length).toBe(1);

    fireEvent.keyDown(cardEl, { key: "ArrowRight", metaKey: true });

    expect(await screen.findByText(/moved in another window/)).toBeTruthy();
    await waitFor(() => expect(vi.mocked(commands.trackerBoard).mock.calls.length).toBe(2));
  });

  it("does not move a card past the end of the board", async () => {
    renderBoard();
    const cardEl = await screen.findByLabelText("Open Backend Engineer");
    fireEvent.keyDown(cardEl, { key: "ArrowLeft", metaKey: true });
    await waitFor(() => expect(commands.trackerBoard).toHaveBeenCalled());
    expect(commands.trackerTransitionStatus).not.toHaveBeenCalled();
  });
});

describe("Tracker card detail (AC3)", () => {
  it("shows the snapshot, the package, the timeline, the next action and notes", async () => {
    renderBoard();
    fireEvent.click(await screen.findByRole("button", { name: "Backend Engineer" }));

    expect(await screen.findByText("Offer snapshot")).toBeTruthy();
    expect(screen.getByText("Package sent")).toBeTruthy();
    expect(screen.getByText(/CV · en · v2/)).toBeTruthy();
    expect(screen.getByText("verified")).toBeTruthy();
    expect(screen.getByText("Timeline")).toBeTruthy();
    expect(screen.getByText(/You · 2026-10-16/)).toBeTruthy();
    expect(screen.getByText(/Saved → Interviewing/)).toBeTruthy();
    expect(screen.getByText(/Status proposal/)).toBeTruthy();
    expect(screen.getByText("follow up on Friday")).toBeTruthy();
    expect(screen.getByText("referred by Ana")).toBeTruthy();
    expect(commands.trackerApplicationDetail).toHaveBeenCalledWith("app_01");
  });

  it("links an https apply_url", async () => {
    renderBoard();
    fireEvent.click(await screen.findByRole("button", { name: "Backend Engineer" }));

    const link = await screen.findByRole("link", { name: "Open the offer" });
    expect(link.getAttribute("href")).toBe("https://jobs.example.com/abc-1");
  });

  it("never links a non-https apply_url", async () => {
    vi.mocked(commands.trackerApplicationDetail).mockImplementation(() =>
      ok({ ...detail, apply_url: "javascript:alert(1)" }),
    );
    renderBoard();
    fireEvent.click(await screen.findByRole("button", { name: "Backend Engineer" }));

    expect(await screen.findByText("javascript:alert(1)")).toBeTruthy();
    expect(screen.queryByRole("link", { name: "Open the offer" })).toBeNull();
  });
});
