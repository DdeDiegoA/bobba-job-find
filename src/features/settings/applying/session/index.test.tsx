import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type SessionAutoView } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SettingsApplyingSession from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    sessionAutoSnapshot: vi.fn(),
    sessionAutoOptIn: vi.fn(),
    sessionAutoRevoke: vi.fn(),
    sessionAutoSetCap: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const board = (over: Partial<SessionAutoView> = {}): SessionAutoView => ({
  source_id: "linkedin",
  name: "LinkedIn",
  reading_opted_in: true,
  state: "off",
  accepted_at: null,
  confirmed_name: null,
  warning_version: null,
  expires_at: null,
  paused_until: null,
  daily_cap: 3,
  used_today: 0,
  max_daily_cap: 10,
  current_warning_version: "v1",
  ...over,
});

let stored: SessionAutoView[];

beforeEach(() => {
  vi.mocked(commands.sessionAutoSnapshot).mockReset();
  vi.mocked(commands.sessionAutoOptIn).mockReset();
  vi.mocked(commands.sessionAutoRevoke).mockReset();
  vi.mocked(commands.sessionAutoSetCap).mockReset();

  stored = [board()];
  vi.mocked(commands.sessionAutoSnapshot).mockImplementation(() => ok(stored));
  vi.mocked(commands.sessionAutoOptIn).mockImplementation((sourceId, typed, warning) => {
    stored = [
      board({
        state: "active",
        accepted_at: "2026-10-17T09:00:00Z",
        confirmed_name: typed,
        warning_version: warning,
        expires_at: "2026-11-16T09:00:00Z",
        source_id: sourceId,
      }),
    ];
    return ok(stored[0]);
  });
  vi.mocked(commands.sessionAutoRevoke).mockImplementation(() => {
    stored = [board()];
    return ok(stored[0]);
  });
  vi.mocked(commands.sessionAutoSetCap).mockImplementation((sourceId, cap) => {
    stored = [{ ...stored[0], source_id: sourceId, daily_cap: cap }];
    return ok(stored[0]);
  });
});

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SettingsApplyingSession />
    </QueryClientProvider>,
  );
}

describe("Settings — Session Board Auto (S11.3, FR-91)", () => {
  it("shows the ToS-risk badge only while Auto is on", async () => {
    stored = [board({ state: "active", accepted_at: "2026-10-17T09:00:00Z" })];
    renderPage();
    expect(await screen.findByText("ToS risk")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Turn off Auto" }));
    await waitFor(() => expect(commands.sessionAutoRevoke).toHaveBeenCalledWith("linkedin"));
    await waitFor(() => expect(screen.queryByText("ToS risk")).toBeNull());
    expect(screen.getByText("Off")).toBeTruthy();
  });

  it("accepts Auto only after the warning, the checkbox and the typed name", async () => {
    renderPage();
    fireEvent.click(await screen.findByRole("button", { name: "Turn on Auto for LinkedIn" }));

    const confirm = screen.getByRole("button", {
      name: "Yes, turn on Auto for LinkedIn",
    }) as HTMLButtonElement;
    expect(confirm.disabled).toBe(true);

    fireEvent.click(screen.getByRole("checkbox", { name: /I have read/ }));
    expect(confirm.disabled).toBe(true);

    fireEvent.change(screen.getByLabelText(/Type LinkedIn to confirm/), {
      target: { value: "Linkedin" },
    });
    expect(confirm.disabled).toBe(true);

    fireEvent.change(screen.getByLabelText(/Type LinkedIn to confirm/), {
      target: { value: "LinkedIn" },
    });
    expect(confirm.disabled).toBe(false);
    fireEvent.click(confirm);

    await waitFor(() =>
      expect(commands.sessionAutoOptIn).toHaveBeenCalledWith("linkedin", "LinkedIn", "v1"),
    );
    expect(await screen.findByText("Auto on")).toBeTruthy();
  });

  it("saves the cap and reports today's count", async () => {
    stored = [board({ state: "active", accepted_at: "2026-10-17T09:00:00Z", used_today: 2 })];
    renderPage();

    fireEvent.change(await screen.findByLabelText("Applications a day"), {
      target: { value: "4" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save cap" }));

    await waitFor(() => expect(commands.sessionAutoSetCap).toHaveBeenCalledWith("linkedin", 4));
    expect(screen.getByText(/2 of 4 used today/)).toBeTruthy();
    expect(screen.getByText(/at least 10 minutes between applications/i)).toBeTruthy();
  });

  it("asks for the session opt-in first on a board that is off", async () => {
    stored = [board({ reading_opted_in: false })];
    renderPage();
    expect(
      await screen.findByText(/Turn on this board's session in Connections first/),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Turn on Auto/ })).toBeNull();
  });

  it("offers a re-confirmation once the opt-in lapsed", async () => {
    stored = [
      board({
        state: "expired",
        accepted_at: "2026-09-17T09:00:00Z",
        expires_at: "2026-10-17T09:00:00Z",
        confirmed_name: "LinkedIn",
      }),
    ];
    renderPage();
    expect(await screen.findByText("Needs re-confirmation")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Confirm again for LinkedIn" })).toBeTruthy();
  });

  it("shows the pause deadline a challenge set", async () => {
    stored = [
      board({
        state: "paused",
        accepted_at: "2026-10-10T09:00:00Z",
        paused_until: "2026-10-24T09:00:00Z",
      }),
    ];
    renderPage();
    expect(await screen.findByText(/resumes on 2026-10-24T09:00:00Z/)).toBeTruthy();
    expect(screen.getByText("ToS risk")).toBeTruthy();
  });
});
