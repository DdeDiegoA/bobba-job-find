import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type SessionApplyView, type SessionOutcome } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SessionAssist from "./session-assist";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    applySessionStart: vi.fn(),
    applySessionFill: vi.fn(),
    applySessionConfirm: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

function session(overrides: Partial<SessionApplyView> = {}): SessionApplyView {
  return {
    application_id: "app_1",
    attempt_id: "session_app_1_2026-10-17T00-00-00Z",
    source_id: "linkedin",
    board: "linkedin",
    url: "https://www.linkedin.com/jobs/view/123/",
    filled: [{ handle: "boba-1", label: "Email address", kind: "email" }],
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

describe("SessionAssist", () => {
  it("always shows the account-ban warning, and starts nothing on its own", () => {
    renderWithClient(<SessionAssist applicationId="app_1" board="linkedin" />);

    expect(screen.getByRole("note").textContent).toMatch(/prohibit automated applying/);
    expect(commands.applySessionStart).not.toHaveBeenCalled();
  });

  it("fills the step in the user's session and reports what it did", async () => {
    vi.mocked(commands.applySessionStart).mockReturnValue(
      ok(
        session({
          handoff: { kind: "unknown_required", label: "Describe your database work" },
        }),
      ) as never,
    );
    renderWithClient(<SessionAssist applicationId="app_1" board="linkedin" />);

    fireEvent.click(screen.getByRole("button", { name: "Open and fill this step" }));

    expect(await screen.findByText(/Filled 1 fields and attached 1/)).toBeTruthy();
    expect(screen.getByText(/Describe your database work/)).toBeTruthy();
    expect(commands.applySessionStart).toHaveBeenCalledWith("app_1");
  });

  it("fills the next step under the same attempt", async () => {
    vi.mocked(commands.applySessionStart).mockReturnValue(ok(session()) as never);
    vi.mocked(commands.applySessionFill).mockReturnValue(
      ok(
        session({
          filled: [{ handle: "boba-9", label: "How many years of Rust?", kind: "text" }],
        }),
      ) as never,
    );
    renderWithClient(<SessionAssist applicationId="app_1" board="linkedin" />);

    fireEvent.click(screen.getByRole("button", { name: "Open and fill this step" }));
    fireEvent.click(await screen.findByRole("button", { name: "Fill the next step" }));

    await waitFor(() =>
      expect(commands.applySessionFill).toHaveBeenCalledWith(
        "app_1",
        "session_app_1_2026-10-17T00-00-00Z",
      ),
    );
  });

  it("confirms the submission and shows the evidence text", async () => {
    const applied: SessionOutcome = {
      kind: "applied",
      evidence: {
        confirmation_text: "Your application was sent to Nimbus.",
        screenshot_blake3: "hash",
      },
    };
    vi.mocked(commands.applySessionStart).mockReturnValue(ok(session()) as never);
    vi.mocked(commands.applySessionConfirm).mockReturnValue(ok(applied) as never);
    renderWithClient(<SessionAssist applicationId="app_1" board="linkedin" />);

    fireEvent.click(screen.getByRole("button", { name: "Open and fill this step" }));
    fireEvent.click(await screen.findByRole("button", { name: "I submitted it" }));

    expect(await screen.findByText(/Your application was sent to Nimbus\./)).toBeTruthy();
    expect(commands.applySessionConfirm).toHaveBeenCalledWith(
      "app_1",
      "session_app_1_2026-10-17T00-00-00Z",
    );
  });

  it("says Needs check when the page showed no confirmation", async () => {
    const needsCheck: SessionOutcome = { kind: "needs_check", screenshot_blake3: "hash" };
    vi.mocked(commands.applySessionStart).mockReturnValue(ok(session()) as never);
    vi.mocked(commands.applySessionConfirm).mockReturnValue(ok(needsCheck) as never);
    renderWithClient(<SessionAssist applicationId="app_1" board="linkedin" />);

    fireEvent.click(screen.getByRole("button", { name: "Open and fill this step" }));
    fireEvent.click(await screen.findByRole("button", { name: "I submitted it" }));

    expect(await screen.findByText(/Needs check/)).toBeTruthy();
  });
});
