import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  commands,
  type FxSettings,
  type NetworkActivity,
  type UpdateSettings,
} from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SettingsPrivacyNetwork from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    networkActivity: vi.fn(),
    fxSettings: vi.fn(),
    fxSetEnabled: vi.fn(),
    updateSettings: vi.fn(),
    updateSetEnabled: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const activity = (over: Partial<NetworkActivity> = {}): NetworkActivity => ({
  groups: [
    {
      purpose: "sources",
      destinations: [
        {
          name: "remotive.com",
          purpose: "sources",
          last_seen: "2026-10-17T09:00:00Z",
          requests: 2,
          note: null,
        },
        {
          name: "boards-api.greenhouse.io",
          purpose: "sources",
          last_seen: null,
          requests: 0,
          note: null,
        },
      ],
    },
    {
      purpose: "email",
      destinations: [
        {
          name: "graph.microsoft.com",
          purpose: "email",
          last_seen: null,
          requests: 0,
          note: "me@example.com",
        },
      ],
    },
  ],
  telemetry: false,
  ...over,
});

const fx = (over: Partial<FxSettings> = {}): FxSettings => ({
  enabled: false,
  target: "EUR",
  as_of: null,
  rates: {},
  ...over,
});

const updates = (over: Partial<UpdateSettings> = {}): UpdateSettings => ({
  enabled: false,
  ...over,
});

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SettingsPrivacyNetwork />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(commands.networkActivity).mockImplementation(() => ok(activity()));
  vi.mocked(commands.fxSettings).mockImplementation(() => ok(fx()));
  // Reading the setting cannot fail, so it returns the value, not a CommandResult.
  vi.mocked(commands.updateSettings).mockImplementation(() => Promise.resolve(updates()));
});

describe("Settings → Privacy → Network activity (NFR-1, AC1)", () => {
  it("groups the destinations by purpose and says there is no telemetry", async () => {
    renderPage();

    expect(await screen.findByRole("heading", { name: "Sources" })).toBeTruthy();
    expect(await screen.findByRole("heading", { name: "Email" })).toBeTruthy();
    expect(await screen.findByText("remotive.com")).toBeTruthy();
    // A configured destination that has not been contacted says so.
    expect(screen.getByText("boards-api.greenhouse.io")).toBeTruthy();
    expect(screen.getAllByText("Configured, not contacted yet").length).toBe(2);
    expect(screen.getByText(/There is no telemetry/)).toBeTruthy();
    // The email account label rides along as a note, never as a second host.
    expect(screen.getByText("me@example.com")).toBeTruthy();
  });

  it("shows the empty state before anything is contacted", async () => {
    vi.mocked(commands.networkActivity).mockImplementation(() => ok(activity({ groups: [] })));
    renderPage();

    expect(await screen.findByText(/Nothing has been contacted yet/)).toBeTruthy();
  });
});

describe("the salary-conversion opt-in (FR-115, AC2)", () => {
  it("is off by default and turning it on asks the core for rates", async () => {
    vi.mocked(commands.fxSetEnabled).mockImplementation(() =>
      ok(fx({ enabled: true, as_of: "2026-10-16", rates: { USD: 1.1 } })),
    );
    renderPage();

    const toggle = await screen.findByRole("checkbox", { name: "Convert salaries" });
    expect((toggle as HTMLInputElement).checked).toBe(false);
    expect(screen.getByText(/Conversions are off/)).toBeTruthy();

    fireEvent.click(toggle);

    await waitFor(() => expect(commands.fxSetEnabled).toHaveBeenCalledWith(true));
    await waitFor(() => expect(screen.getByText(/Conversions are on/)).toBeTruthy());
  });

  it("labels an enabled conversion with the date of its rate", async () => {
    vi.mocked(commands.fxSettings).mockImplementation(() =>
      ok(fx({ enabled: true, as_of: "2026-10-16", rates: { USD: 1.1 } })),
    );
    renderPage();

    expect(await screen.findByText(/Rates from Oct 16, 2026, per 1 EUR/)).toBeTruthy();
  });
});

describe("the update-check opt-in (S10.5, NFR-1, NFR-19)", () => {
  it("is off by default and turning it on asks the core to check", async () => {
    vi.mocked(commands.updateSetEnabled).mockImplementation(() => ok(updates({ enabled: true })));
    renderPage();

    const toggle = await screen.findByRole("checkbox", { name: "Check for updates" });
    expect((toggle as HTMLInputElement).checked).toBe(false);
    expect(screen.getByText(/never contacts the update server/)).toBeTruthy();

    fireEvent.click(toggle);

    await waitFor(() => expect(commands.updateSetEnabled).toHaveBeenCalledWith(true));
    await waitFor(() => expect(screen.getByText(/Update checks are on/)).toBeTruthy());
  });
});
