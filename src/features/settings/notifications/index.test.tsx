import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type NotificationSettings } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SettingsNotifications from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    notificationsSnapshot: vi.fn(),
    notificationsSave: vi.fn(),
    autostartStatus: vi.fn(),
    autostartSet: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const stored = (over: Partial<NotificationSettings> = {}): NotificationSettings => ({
  events: { digest: true, outbox: true, errors: true },
  top_matches_shown: 3,
  notify_when_empty: false,
  quiet_hours: { enabled: false, from: 22, to: 7, notify_during: false },
  ...over,
});

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SettingsNotifications />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.notificationsSnapshot).mockReset();
  vi.mocked(commands.notificationsSave).mockReset();
  vi.mocked(commands.autostartStatus).mockReset();
  vi.mocked(commands.autostartSet).mockReset();
  vi.mocked(commands.notificationsSnapshot).mockImplementation(() => ok(stored()));
  vi.mocked(commands.notificationsSave).mockImplementation((settings) => ok(settings));
  vi.mocked(commands.autostartStatus).mockResolvedValue({ enabled: false });
  vi.mocked(commands.autostartSet).mockImplementation((enabled) => ok({ enabled }));
});

describe("Settings → Notifications (S5.3, FR-107)", () => {
  it("shows the stored choices", async () => {
    vi.mocked(commands.notificationsSnapshot).mockImplementation(() =>
      ok(
        stored({
          events: { digest: true, outbox: false, errors: false },
          top_matches_shown: 5,
          quiet_hours: { enabled: true, from: 22, to: 7, notify_during: true },
        }),
      ),
    );
    renderPage();

    const errors = await screen.findByLabelText("Sources that keep failing");
    expect((errors as HTMLInputElement).checked).toBe(false);
    expect((screen.getByLabelText("The daily digest") as HTMLInputElement).checked).toBe(true);
    expect((screen.getByLabelText("Use quiet hours") as HTMLInputElement).checked).toBe(true);
    expect(
      (screen.getByLabelText("Still notify me during quiet hours") as HTMLInputElement).checked,
    ).toBe(true);
    expect((screen.getByLabelText("From") as HTMLSelectElement).value).toBe("22");
    expect((screen.getByLabelText("Until") as HTMLSelectElement).value).toBe("7");
    expect((screen.getByLabelText("Top matches to name") as HTMLInputElement).value).toBe("5");
  });

  it("saves the edited choices and says so", async () => {
    renderPage();

    const digest = await screen.findByLabelText("The daily digest");
    const cap = screen.getByLabelText("Top matches to name");
    const quiet = screen.getByLabelText("Use quiet hours");

    fireEvent.click(digest);
    fireEvent.change(cap, { target: { value: "2" } });
    fireEvent.click(quiet);
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(vi.mocked(commands.notificationsSave)).toHaveBeenCalledWith({
        events: { digest: false, outbox: true, errors: true },
        top_matches_shown: 2,
        notify_when_empty: false,
        quiet_hours: { enabled: true, from: 22, to: 7, notify_during: false },
      }),
    );
    expect(await screen.findByText("Saved")).toBeTruthy();
  });

  it("says so when the settings cannot be loaded", async () => {
    vi.mocked(commands.notificationsSnapshot).mockImplementation(() =>
      Promise.resolve({
        status: "error" as const,
        error: { code: "internal", message_key: "errors.store.locked", details: null },
      }),
    );
    renderPage();

    // The core's typed key is what the page shows (AD-3), not a raw string.
    expect(await screen.findByText(/can't reach your data/i)).toBeTruthy();
  });
});

describe("Launch at login (FX14, FR-44)", () => {
  it("reflects the OS state the core reports", async () => {
    vi.mocked(commands.autostartStatus).mockResolvedValue({ enabled: true });
    renderPage();

    const toggle = await screen.findByRole("checkbox", { name: "Start Boba when I log in" });
    expect((toggle as HTMLInputElement).checked).toBe(true);
  });

  it("sets it in the core and shows the refreshed state", async () => {
    renderPage();

    const toggle = await screen.findByRole("checkbox", { name: "Start Boba when I log in" });
    expect((toggle as HTMLInputElement).checked).toBe(false);

    // The OS now starts Boba at login; the page re-reads that, never assumes it.
    vi.mocked(commands.autostartStatus).mockResolvedValue({ enabled: true });
    fireEvent.click(toggle);

    await waitFor(() => expect(vi.mocked(commands.autostartSet)).toHaveBeenCalledWith(true));
    await waitFor(() => expect((toggle as HTMLInputElement).checked).toBe(true));
    expect(vi.mocked(commands.autostartStatus)).toHaveBeenCalledTimes(2);
  });

  it("shows a translated error when the core refuses", async () => {
    vi.mocked(commands.autostartSet).mockRejectedValue({
      code: "locked",
      message_key: "errors.store.locked",
      details: null,
    });
    renderPage();

    fireEvent.click(await screen.findByRole("checkbox", { name: "Start Boba when I log in" }));

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toMatch(/can't reach your data/i);
  });

  it("is a native, focusable checkbox with its own label (keyboard-operable)", async () => {
    renderPage();

    const toggle = await screen.findByRole("checkbox", { name: "Start Boba when I log in" });
    expect(toggle.tagName).toBe("INPUT");
    toggle.focus();
    expect(document.activeElement).toBe(toggle);
  });
});
