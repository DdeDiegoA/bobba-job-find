import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  type AnswerBankView,
  type AtsOutcome,
  type AtsSessionView,
  commands,
} from "@/lib/ipc/bindings";
import "@/lib/i18n";
import AnswerApprovePanel from "./approve-panel";
import AtsAssist from "./ats-assist";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    applyAtsOptedIn: vi.fn(),
    applyAtsOptIn: vi.fn(),
    applyAtsStart: vi.fn(),
    applyAtsConfirm: vi.fn(),
    applyAnswers: vi.fn(),
    applyAnswerApprove: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

function session(overrides: Partial<AtsSessionView> = {}): AtsSessionView {
  return {
    application_id: "app_1",
    attempt_id: "ats_app_1_2026-10-17T00-00-00Z",
    url: "https://jobs.lever.co/nimbus/abc",
    provider: "lever",
    filled: [{ handle: "boba-1", label: "Email", kind: "email" }],
    attached: ["cv"],
    unanswered: [],
    handoff: null,
    ...overrides,
  };
}

function renderWithClient(ui: React.ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("AtsAssist", () => {
  it("warns before it may drive the user's browser, then turns filling on", async () => {
    vi.mocked(commands.applyAtsOptedIn).mockReturnValue(ok(false) as never);
    vi.mocked(commands.applyAtsOptIn).mockReturnValue(ok(true) as never);
    renderWithClient(<AtsAssist applicationId="app_1" />);

    expect(await screen.findByText(/your own Chrome/)).toBeTruthy();
    expect(commands.applyAtsStart).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Turn on form filling" }));

    await waitFor(() => expect(commands.applyAtsOptIn).toHaveBeenCalledWith(true));
    expect(await screen.findByRole("button", { name: "Open and fill the form" })).toBeTruthy();
  });

  it("reports how much it filled and where it handed control back", async () => {
    vi.mocked(commands.applyAtsOptedIn).mockReturnValue(ok(true) as never);
    vi.mocked(commands.applyAtsStart).mockReturnValue(
      ok(
        session({
          handoff: { kind: "unknown_required", label: "Describe your database work" },
        }),
      ) as never,
    );
    renderWithClient(<AtsAssist applicationId="app_1" />);

    fireEvent.click(await screen.findByRole("button", { name: "Open and fill the form" }));

    expect(await screen.findByText(/Filled 1 fields and attached 1/)).toBeTruthy();
    expect(screen.getByText(/Describe your database work/)).toBeTruthy();
    await waitFor(() => expect(commands.applyAtsStart).toHaveBeenCalledWith("app_1"));
  });

  it("confirms the submission and shows the evidence text", async () => {
    const applied: AtsOutcome = {
      kind: "applied",
      evidence: {
        confirmation_text: "Thank you for applying to Nimbus.",
        screenshot_blake3: "hash",
      },
    };
    vi.mocked(commands.applyAtsOptedIn).mockReturnValue(ok(true) as never);
    vi.mocked(commands.applyAtsStart).mockReturnValue(ok(session()) as never);
    vi.mocked(commands.applyAtsConfirm).mockReturnValue(ok(applied) as never);
    renderWithClient(<AtsAssist applicationId="app_1" />);

    fireEvent.click(await screen.findByRole("button", { name: "Open and fill the form" }));
    fireEvent.click(await screen.findByRole("button", { name: "I submitted it" }));

    expect(await screen.findByText(/Thank you for applying to Nimbus\./)).toBeTruthy();
    expect(commands.applyAtsConfirm).toHaveBeenCalledWith(
      "app_1",
      "ats_app_1_2026-10-17T00-00-00Z",
    );
  });

  it("says Needs check when the page showed no confirmation", async () => {
    const needsCheck: AtsOutcome = { kind: "needs_check", screenshot_blake3: "hash" };
    vi.mocked(commands.applyAtsOptedIn).mockReturnValue(ok(true) as never);
    vi.mocked(commands.applyAtsStart).mockReturnValue(ok(session()) as never);
    vi.mocked(commands.applyAtsConfirm).mockReturnValue(ok(needsCheck) as never);
    renderWithClient(<AtsAssist applicationId="app_1" />);

    fireEvent.click(await screen.findByRole("button", { name: "Open and fill the form" }));
    fireEvent.click(await screen.findByRole("button", { name: "I submitted it" }));

    expect(await screen.findByText(/Needs check/)).toBeTruthy();
  });
});

describe("AnswerApprovePanel", () => {
  function bank(approved: boolean): AnswerBankView {
    return {
      application_id: "app_1",
      answers: [
        {
          id: "ans_1",
          question: "When can you start?",
          answer: "Two weeks",
          class: "other",
          needs_review: false,
          approved,
        },
      ],
    };
  }

  it("approves a drafted answer with the user's edit", async () => {
    vi.mocked(commands.applyAnswers).mockReturnValue(ok(bank(false)) as never);
    vi.mocked(commands.applyAnswerApprove).mockReturnValue(ok(bank(true)) as never);
    renderWithClient(<AnswerApprovePanel applicationId="app_1" />);

    const editor = await screen.findByLabelText("When can you start?");
    fireEvent.change(editor, { target: { value: "Next month" } });
    fireEvent.click(screen.getByRole("button", { name: "Approve answer" }));

    await waitFor(() =>
      expect(commands.applyAnswerApprove).toHaveBeenCalledWith("app_1", "ans_1", "Next month"),
    );
    expect(await screen.findByText("Approved")).toBeTruthy();
  });

  it("confirms an answer as it stands", async () => {
    vi.mocked(commands.applyAnswers).mockReturnValue(ok(bank(false)) as never);
    vi.mocked(commands.applyAnswerApprove).mockReturnValue(ok(bank(true)) as never);
    renderWithClient(<AnswerApprovePanel applicationId="app_1" />);

    fireEvent.click(await screen.findByRole("button", { name: "Approve answer" }));

    await waitFor(() =>
      expect(commands.applyAnswerApprove).toHaveBeenCalledWith("app_1", "ans_1", null),
    );
  });
});
