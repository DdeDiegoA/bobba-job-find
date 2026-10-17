import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type SessionSnapshotView } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SessionOptin from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    sessionSnapshot: vi.fn(),
    sessionOptIn: vi.fn(),
    sessionOptOut: vi.fn(),
    sessionSignOut: vi.fn(),
    sessionResume: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const board = (over: Record<string, unknown> = {}) => ({
  source_id: "linkedin",
  name: "LinkedIn",
  enabled: false,
  opted_in: false,
  accepted_at: null,
  accepted_warning_version: null,
  pause: null,
  weak_ack: false,
  ...over,
});

const snapshot = (over: Partial<SessionSnapshotView> = {}): SessionSnapshotView =>
  ({
    warning_version: "v1",
    cookie_protection: "macos_keychain",
    requires_extra_ack: false,
    boards: [board()],
    ...over,
  }) as SessionSnapshotView;

function renderCard() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SessionOptin sourceId="linkedin" sourceName="LinkedIn" />
    </QueryClientProvider>,
  );
}

const openWarning = async () => {
  fireEvent.click(await screen.findByRole("button", { name: "Turn on LinkedIn" }));
};

beforeEach(() => {
  vi.mocked(commands.sessionSnapshot).mockReset();
  vi.mocked(commands.sessionOptIn).mockReset();
  vi.mocked(commands.sessionSignOut).mockReset();
  vi.mocked(commands.sessionOptOut).mockReset();
  vi.mocked(commands.sessionResume).mockReset();
});

describe("Session opt-in card (FR-30, AC1/AC2)", () => {
  it("shows the account-ban risk and the platform cookie sentence only after asking", async () => {
    vi.mocked(commands.sessionSnapshot).mockImplementation(() => ok(snapshot()));
    renderCard();

    expect(await screen.findByText("LinkedIn")).toBeTruthy();
    expect(screen.queryByText(/restricted or banned/)).toBeNull();

    await openWarning();

    expect(await screen.findByText(/restricted or banned/)).toBeTruthy();
    expect(screen.getByText(/macOS Keychain/)).toBeTruthy();
    expect(screen.getByText(/Sign out \/ clear session/)).toBeTruthy();
    expect(screen.getByText("Warning v1")).toBeTruthy();
  });

  it("records the version only once the warning is acknowledged", async () => {
    vi.mocked(commands.sessionSnapshot).mockImplementation(() => ok(snapshot()));
    vi.mocked(commands.sessionOptIn).mockImplementation(() => ok(board({ opted_in: true })));
    renderCard();
    await openWarning();

    const confirm = screen.getByRole("button", {
      name: "Yes, turn on LinkedIn",
    }) as HTMLButtonElement;
    expect(confirm.disabled).toBe(true);

    fireEvent.click(screen.getByRole("checkbox", { name: /I have read and understand/ }));
    expect(confirm.disabled).toBe(false);
    fireEvent.click(confirm);

    await waitFor(() =>
      expect(commands.sessionOptIn).toHaveBeenCalledWith("linkedin", "v1", false),
    );
  });

  it("asks for the separate acknowledgement on Linux without a keyring (AC2)", async () => {
    vi.mocked(commands.sessionSnapshot).mockImplementation(() =>
      ok(snapshot({ cookie_protection: "linux_weak", requires_extra_ack: true })),
    );
    vi.mocked(commands.sessionOptIn).mockImplementation(() => ok(board({ opted_in: true })));
    renderCard();
    await openWarning();

    expect(screen.getByText(/No keyring is available/)).toBeTruthy();
    const confirm = screen.getByRole("button", {
      name: "Yes, turn on LinkedIn",
    }) as HTMLButtonElement;
    fireEvent.click(screen.getByRole("checkbox", { name: /I have read and understand/ }));
    expect(confirm.disabled).toBe(true);

    fireEvent.click(screen.getByRole("checkbox", { name: /protected only weakly/ }));
    expect(confirm.disabled).toBe(false);
    fireEvent.click(confirm);

    await waitFor(() => expect(commands.sessionOptIn).toHaveBeenCalledWith("linkedin", "v1", true));
  });

  it("signs out of one source's session and says so (AC3)", async () => {
    vi.mocked(commands.sessionSnapshot).mockImplementation(() =>
      ok(
        snapshot({ boards: [board({ opted_in: true, enabled: true, accepted_at: "2026-10-17" })] }),
      ),
    );
    vi.mocked(commands.sessionSignOut).mockImplementation(() => ok(board({ opted_in: true })));
    renderCard();

    fireEvent.click(await screen.findByRole("button", { name: "Sign out / clear session" }));

    await waitFor(() => expect(commands.sessionSignOut).toHaveBeenCalledWith("linkedin"));
    expect(await screen.findByText("Session cleared.")).toBeTruthy();
  });

  it("offers a resume for a paused source (FR-30)", async () => {
    vi.mocked(commands.sessionSnapshot).mockImplementation(() =>
      ok(
        snapshot({
          boards: [
            board({
              opted_in: true,
              enabled: false,
              pause: { kind: "captcha", since: "2026-10-17T06:00:00.000Z" },
            }),
          ],
        }),
      ),
    );
    vi.mocked(commands.sessionResume).mockImplementation(() => ok(board({ opted_in: true })));
    renderCard();

    expect(await screen.findByText(/Paused after a CAPTCHA/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "I've resolved it — resume" }));
    await waitFor(() => expect(commands.sessionResume).toHaveBeenCalledWith("linkedin"));
  });
});
