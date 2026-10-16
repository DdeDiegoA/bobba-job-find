import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type Preferences, type PreferencesSnapshot } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SettingsPreferences from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: { preferencesSnapshot: vi.fn(), preferencesSave: vi.fn() },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const values = (over: Partial<Preferences> = {}): Preferences => ({
  roles: [],
  seniority_min: null,
  seniority_max: null,
  salary_floor: null,
  locations: [],
  timezone_window: { from: 8, to: 18 },
  work_modes: { remote: true, hybrid: false, onsite: false },
  working_languages: ["en"],
  deal_breaker_keywords: [],
  deal_breaker_notes: "",
  excluded_companies: [],
  ...over,
});

/** The core's rule, faked (version bump + stale) so the UI can be tested alone. */
let stored: PreferencesSnapshot;
function fakeCore(snapshot: PreferencesSnapshot) {
  stored = snapshot;
  vi.mocked(commands.preferencesSnapshot).mockImplementation(() => ok(stored));
  vi.mocked(commands.preferencesSave).mockImplementation((input) => {
    if (JSON.stringify(input) !== JSON.stringify(stored.values)) {
      stored = { version: stored.version + 1, values: input, scores_stale: true };
    }
    return ok(stored);
  });
}

const saveButton = () =>
  screen.getByRole("button", { name: "Save preferences" }) as HTMLButtonElement;

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SettingsPreferences />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.preferencesSnapshot).mockReset();
  vi.mocked(commands.preferencesSave).mockReset();
});

describe("Settings — Preferences page (FR-7, AC2)", () => {
  it("edits the same fields the wizard asked for", async () => {
    fakeCore({ version: 1, values: values(), scores_stale: false });
    renderPage();

    expect(await screen.findByText("Salary floor")).toBeTruthy();
    expect(screen.getByText("Excluded companies")).toBeTruthy();
    expect(screen.getByText("v1")).toBeTruthy();
  });

  it("saves only when something changed, and the core then bumps the version (AC2)", async () => {
    fakeCore({
      version: 2,
      values: values({ roles: ["Data Engineer"] }),
      scores_stale: false,
    });
    renderPage();

    await screen.findByText("v2");
    expect(saveButton().disabled).toBe(true);

    await addTo("Roles and titles", "Analytics Engineer");
    expect(saveButton().disabled).toBe(false);
    expect(commands.preferencesSave).not.toHaveBeenCalled();
    expect(stored.version).toBe(2);

    fireEvent.click(saveButton());

    await waitFor(() => expect(screen.getByText("v3")).toBeTruthy());
    expect(stored.values.roles).toContain("Analytics Engineer");
  });

  it("says scores are out of date until a re-score consumed the version (FR-14)", async () => {
    fakeCore({ version: 3, values: values({ locations: ["Lisbon"] }), scores_stale: true });
    const page = renderPage();
    expect(await screen.findByText(/Scores are out of date/)).toBeTruthy();
    page.unmount();

    // A re-score (S6.3) flips the flag in the core; the page then stays quiet.
    fakeCore({ version: 3, values: values({ locations: ["Lisbon"] }), scores_stale: false });
    renderPage();
    expect(await screen.findByText("v3")).toBeTruthy();
    expect(screen.queryByText(/Scores are out of date/)).toBeNull();
  });
});

async function addTo(label: string, value: string) {
  const input = await screen.findByLabelText(label);
  fireEvent.change(input, { target: { value } });
  fireEvent.click(screen.getByRole("button", { name: `Add ${label}` }));
}
