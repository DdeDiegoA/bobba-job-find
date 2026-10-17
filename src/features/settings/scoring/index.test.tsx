import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type Preferences, type PreferencesSnapshot } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SettingsScoring from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    preferencesSnapshot: vi.fn(),
    preferencesSave: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const DEFAULTS = {
  skills: 35,
  experience: 25,
  semantic: 15,
  location: 10,
  education: 10,
  language: 5,
  compensation: 30,
  company: 25,
  terms: 15,
  legitimacy: 20,
  alignment: 10,
  fit: 65,
  quality: 35,
};

const values = (over: Partial<Preferences> = {}): Preferences => ({
  roles: ["Data engineer"],
  seniority_min: null,
  seniority_max: null,
  salary_floor: null,
  locations: ["Madrid"],
  timezone_window: { from: 8, to: 18 },
  work_modes: { remote: true, hybrid: false, onsite: false },
  working_languages: ["en"],
  deal_breaker_keywords: [],
  deal_breaker_notes: "",
  excluded_companies: [],
  weights: { ...DEFAULTS },
  ...over,
});

const snapshot = (over: Partial<PreferencesSnapshot> = {}): PreferencesSnapshot => ({
  version: 4,
  scores_stale: false,
  values: values(),
  ...over,
});

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SettingsScoring />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.preferencesSnapshot).mockReset();
  vi.mocked(commands.preferencesSave).mockReset();
  vi.mocked(commands.preferencesSnapshot).mockImplementation(() => ok(snapshot()));
  vi.mocked(commands.preferencesSave).mockImplementation((next) =>
    ok(snapshot({ version: 5, scores_stale: true, values: next })),
  );
});

describe("Settings → Scoring (S6.8)", () => {
  // AC1: a weights edit saves a new Preferences document, which is a new prefs
  // version in the core and therefore a re-score.
  it("saves a changed weight with the rest of the preferences", async () => {
    renderPage();

    // Wait for the stored document: the screen edits what the core holds.
    expect(await screen.findByText("v4")).toBeTruthy();
    const skills = screen.getByLabelText("Skills (F1)");
    expect((skills as HTMLInputElement).value).toBe("35");

    fireEvent.change(skills, { target: { value: "50" } });
    fireEvent.click(screen.getByRole("button", { name: "Save weights" }));

    await waitFor(() =>
      expect(vi.mocked(commands.preferencesSave)).toHaveBeenCalledWith(
        expect.objectContaining({
          roles: ["Data engineer"],
          weights: expect.objectContaining({ skills: 50, fit: 65 }),
        }),
      ),
    );
    expect(await screen.findByText(/Weights saved/)).toBeTruthy();
  });

  it("resets every weight to the PRD defaults, axis weights included", async () => {
    vi.mocked(commands.preferencesSnapshot).mockImplementation(() =>
      ok(
        snapshot({
          values: values({ weights: { ...DEFAULTS, skills: 90, fit: 20, quality: 80 } }),
        }),
      ),
    );
    renderPage();

    expect(await screen.findByText("v4")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Reset to defaults" }));

    await waitFor(() =>
      expect(vi.mocked(commands.preferencesSave)).toHaveBeenCalledWith(
        expect.objectContaining({ weights: DEFAULTS }),
      ),
    );
  });

  // The core refuses an axis with no weight; the screen must not send it.
  it("refuses to save a document with an empty Fit axis", async () => {
    renderPage();

    expect(await screen.findByText("v4")).toBeTruthy();
    for (const label of [
      "Skills (F1)",
      "Experience (F2)",
      "Semantic match (F3)",
      "Location (F4)",
      "Education (F5)",
      "Language (F6)",
    ]) {
      fireEvent.change(await screen.findByLabelText(label), { target: { value: "0" } });
    }

    expect(await screen.findByRole("alert")).toBeTruthy();
    const save = screen.getByRole("button", { name: "Save weights" }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    expect(vi.mocked(commands.preferencesSave)).not.toHaveBeenCalled();
  });
});
