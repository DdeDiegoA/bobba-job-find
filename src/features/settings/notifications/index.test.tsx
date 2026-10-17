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
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const stored = (over: Partial<NotificationSettings> = {}): NotificationSettings => ({
  events: { digest: true, outbox: true, needs_you: true, errors: true },
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
  vi.mocked(commands.notificationsSnapshot).mockImplementation(() => ok(stored()));
  vi.mocked(commands.notificationsSave).mockImplementation((settings) => ok(settings));
});

describe("Settings → Notifications (S5.3, FR-107)", () => {
  it("shows the stored choices", async () => {
    vi.mocked(commands.notificationsSnapshot).mockImplementation(() =>
      ok(
        stored({
          events: { digest: true, outbox: false, needs_you: true, errors: false },
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
        events: { digest: false, outbox: true, needs_you: true, errors: true },
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
