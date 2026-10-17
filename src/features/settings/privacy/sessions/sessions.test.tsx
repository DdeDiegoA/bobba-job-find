import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type SessionSnapshotView } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SettingsPrivacySessions from "./index";

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
  enabled: true,
  opted_in: true,
  accepted_at: "2026-10-17T06:00:00.000Z",
  accepted_warning_version: "v1",
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

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SettingsPrivacySessions />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.sessionSnapshot).mockReset();
  vi.mocked(commands.sessionSignOut).mockReset();
  vi.mocked(commands.sessionResume).mockReset();
});

describe("Settings → Privacy → Sessions (FR-30, AC4)", () => {
  it("states the retention for each session board source and how cookies are protected", async () => {
    vi.mocked(commands.sessionSnapshot).mockImplementation(() => ok(snapshot()));
    renderPage();

    expect(
      await screen.findByText(
        "Cookies are kept until you sign out of this source or reset everything.",
      ),
    ).toBeTruthy();
    expect(screen.getByText(/Chrome protects these cookies with the macOS Keychain/)).toBeTruthy();
    expect(screen.getByText(/On, accepted/)).toBeTruthy();
  });

  it("names the weaker protection on Linux without a keyring", async () => {
    vi.mocked(commands.sessionSnapshot).mockImplementation(() =>
      ok(snapshot({ cookie_protection: "linux_weak", requires_extra_ack: true })),
    );
    renderPage();

    expect(await screen.findByText(/weaker, machine-local protection/)).toBeTruthy();
  });

  it("signs out of one source and confirms the session is gone (AC3)", async () => {
    vi.mocked(commands.sessionSnapshot).mockImplementation(() => ok(snapshot()));
    vi.mocked(commands.sessionSignOut).mockImplementation(() => ok(board()));
    renderPage();

    fireEvent.click(await screen.findByRole("button", { name: "Sign out / clear session" }));

    await waitFor(() => expect(commands.sessionSignOut).toHaveBeenCalledWith("linkedin"));
    expect(await screen.findByText("Session cleared.")).toBeTruthy();
  });

  it("a paused source says why and offers a resume", async () => {
    vi.mocked(commands.sessionSnapshot).mockImplementation(() =>
      ok(
        snapshot({
          boards: [
            board({
              enabled: false,
              pause: { kind: "unusual_activity", since: "2026-10-17T06:00:00.000Z" },
            }),
          ],
        }),
      ),
    );
    vi.mocked(commands.sessionResume).mockImplementation(() => ok(board()));
    renderPage();

    expect(await screen.findByText(/unusual activity/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "I've resolved it — resume" }));
    await waitFor(() => expect(commands.sessionResume).toHaveBeenCalledWith("linkedin"));
  });

  it("says when no session board source keeps cookies", async () => {
    vi.mocked(commands.sessionSnapshot).mockImplementation(() => ok(snapshot({ boards: [] })));
    renderPage();

    expect(await screen.findByText("No session board source is keeping cookies.")).toBeTruthy();
  });
});
