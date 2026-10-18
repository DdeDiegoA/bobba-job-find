import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { type ClassifyOutcome, commands, type StatusProposalView } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import Today from "./index";

// The three older leaves have their own render tests; this file is about the
// container's registry, so they stand in as plain headings. Needs you stays
// real — FX13 is the slice that mounts it.
vi.mock("./matches", () => ({ default: () => <h2>Matches</h2> }));
vi.mock("./outbox", () => ({ default: () => <h2>Outbox</h2> }));
vi.mock("./run-report", () => ({ default: () => <h2>Last search run</h2> }));

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    statusProposalsDisclosure: vi.fn(),
    statusProposalsSaveSettings: vi.fn(),
    statusProposalsClassify: vi.fn(),
    statusProposalsPending: vi.fn(),
    statusProposalsApprove: vi.fn(),
    statusProposalsApproveMany: vi.fn(),
    statusProposalsDismiss: vi.fn(),
    statusProposalsUndo: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const proposal = (over: Partial<StatusProposalView> = {}): StatusProposalView => ({
  id: "sp_1",
  class: "rejection",
  confidence: 0.97,
  quoted_line: "Unfortunately, we cannot proceed.",
  untrusted_origin: false,
  message_id: "eml_1",
  message_subject: "Your application",
  message_from: "recruiting@acme.com",
  received_at: "2026-10-17T09:00:00Z",
  application: {
    application_id: "app_1",
    company: "Acme",
    title: "Backend Engineer",
    status: "applied",
  },
  candidates: [],
  needs_pick: false,
  to_status: "rejected",
  created_at: "2026-10-17T09:00:00Z",
  ...over,
});

const classifyOutcome = (over: Partial<ClassifyOutcome> = {}): ClassifyOutcome => ({
  messages: 1,
  classified: 1,
  proposals: 1,
  auto_applied: 0,
  auto_applied_ids: [],
  uncertain: 0,
  dropped: 0,
  llm_asked: false,
  ...over,
});

function renderToday() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <Today />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(commands.statusProposalsDisclosure).mockReturnValue(
    ok({ mode: "deterministic", acknowledged: true, auto_confirm: false, needs_disclosure: false }),
  );
  vi.mocked(commands.statusProposalsPending).mockReturnValue(ok([]));
});

describe("Today container (FX13, stub sweep #5)", () => {
  it("mounts every registered panel, Needs you included, under one page heading", async () => {
    renderToday();

    expect(screen.getByRole("heading", { level: 1, name: "Today" })).toBeTruthy();
    expect(screen.getByRole("heading", { level: 2, name: "Matches" })).toBeTruthy();
    expect(screen.getByRole("heading", { level: 2, name: "Outbox" })).toBeTruthy();
    expect(screen.getByRole("heading", { level: 2, name: "Last search run" })).toBeTruthy();
    expect(await screen.findByRole("heading", { level: 2, name: "Needs you" })).toBeTruthy();
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
  });

  it("shows a pending proposal and moves nothing until I approve it", async () => {
    vi.mocked(commands.statusProposalsPending).mockReturnValue(ok([proposal()]));
    vi.mocked(commands.statusProposalsApprove).mockReturnValue(
      ok({
        proposal_id: "sp_1",
        application_id: "app_1",
        from_status: "applied",
        to_status: "rejected",
      }),
    );

    renderToday();

    const approve = await screen.findByRole("button", { name: "Approve" });
    expect(screen.getByRole("button", { name: "Dismiss" })).toBeTruthy();
    expect(commands.statusProposalsApprove).not.toHaveBeenCalled();
    expect(commands.statusProposalsDismiss).not.toHaveBeenCalled();

    fireEvent.click(approve);

    await waitFor(() => expect(commands.statusProposalsApprove).toHaveBeenCalledWith("sp_1", null));
  });

  it("classifies replies deterministically with no LLM engine (Limited mode)", async () => {
    vi.mocked(commands.statusProposalsClassify).mockReturnValue(ok(classifyOutcome()));

    renderToday();

    fireEvent.click(await screen.findByRole("button", { name: "Check replies" }));

    await waitFor(() => expect(commands.statusProposalsClassify).toHaveBeenCalled());
  });
});
