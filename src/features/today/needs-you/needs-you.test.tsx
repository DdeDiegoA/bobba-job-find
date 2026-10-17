import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  type ClassifyOutcome,
  commands,
  type DisclosureView,
  type StatusProposalView,
} from "@/lib/ipc/bindings";
import "@/lib/i18n";
import TodayNeedsYou from "./index";

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

const disclosure = (over: Partial<DisclosureView> = {}): DisclosureView => ({
  mode: "llm",
  acknowledged: true,
  auto_confirm: false,
  needs_disclosure: false,
  ...over,
});

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

const uncertainOutcome = (over: Partial<ClassifyOutcome> = {}): ClassifyOutcome => ({
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

function renderPanel() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <TodayNeedsYou />
    </QueryClientProvider>,
  );
}

describe("Today needs-you queue", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(commands.statusProposalsDisclosure).mockReturnValue(ok(disclosure()));
    vi.mocked(commands.statusProposalsPending).mockReturnValue(ok([]));
  });

  it("asks before reading email the first time, and offers deterministic-only", async () => {
    vi.mocked(commands.statusProposalsDisclosure).mockReturnValue(
      ok(disclosure({ acknowledged: false, needs_disclosure: true })),
    );
    vi.mocked(commands.statusProposalsSaveSettings).mockReturnValue(
      ok(disclosure({ mode: "deterministic", acknowledged: true, needs_disclosure: false })),
    );

    renderPanel();

    fireEvent.click(
      await screen.findByRole("button", { name: /deterministic classification only/i }),
    );

    await waitFor(() =>
      expect(commands.statusProposalsSaveSettings).toHaveBeenCalledWith(
        "deterministic",
        false,
        true,
      ),
    );
  });

  it("approves a proposal and moves its card", async () => {
    vi.mocked(commands.statusProposalsPending).mockReturnValue(ok([proposal()]));
    vi.mocked(commands.statusProposalsApprove).mockReturnValue(
      ok({
        proposal_id: "sp_1",
        application_id: "app_1",
        from_status: "applied",
        to_status: "rejected",
      }),
    );

    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: /^approve$/i }));

    await waitFor(() => expect(commands.statusProposalsApprove).toHaveBeenCalledWith("sp_1", null));
  });

  it("renders the quoted line as plain text, never as markup", async () => {
    vi.mocked(commands.statusProposalsPending).mockReturnValue(
      ok([
        proposal({
          untrusted_origin: true,
          quoted_line: "<b>Unfortunately</b>, we cannot proceed.",
        }),
      ]),
    );

    renderPanel();

    const card = await screen.findByTestId("proposal-sp_1");
    expect(card.textContent).toContain("<b>Unfortunately</b>, we cannot proceed.");
    expect(card.querySelector("b")).toBeNull();
  });

  it("reports the ambiguous proposals a batch could not approve", async () => {
    vi.mocked(commands.statusProposalsPending).mockReturnValue(
      ok([
        proposal(),
        proposal({ id: "sp_2", needs_pick: true, application: null, candidates: [] }),
      ]),
    );
    vi.mocked(commands.statusProposalsApproveMany).mockReturnValue(
      ok({ approved: ["sp_1"], needs_pick: ["sp_2"], failed: [] }),
    );

    renderPanel();

    fireEvent.click(await screen.findByRole("checkbox", { name: /select all/i }));
    fireEvent.click(await screen.findByRole("button", { name: /approve selected/i }));

    await waitFor(() =>
      expect(commands.statusProposalsApproveMany).toHaveBeenCalledWith(["sp_1", "sp_2"]),
    );
    expect(await screen.findByText(/need you to pick the application first/i)).toBeTruthy();
  });

  it("offers undo right after an automatic change", async () => {
    vi.mocked(commands.statusProposalsClassify).mockReturnValue(
      ok(uncertainOutcome({ auto_applied: 1, auto_applied_ids: ["sp_auto"] })),
    );
    vi.mocked(commands.statusProposalsUndo).mockReturnValue(ok(null));

    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: /check replies/i }));
    fireEvent.click(await screen.findByRole("button", { name: /undo/i }));

    await waitFor(() => expect(commands.statusProposalsUndo).toHaveBeenCalledWith("sp_auto"));
  });
});
