import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type Preferences, type PreferencesSnapshot } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import Settings from "./index";

// The container and the route are what this file tests; the pages bring their
// own IPC surface with them. Audit is stubbed, Scoring and Preferences stay real.
vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    preferencesSnapshot: vi.fn(),
    preferencesSave: vi.fn(),
    embeddingsStatus: vi.fn(),
    embeddingsOptIn: vi.fn(),
    embeddingsOptOut: vi.fn(),
  },
}));
vi.mock("./privacy/audit", () => ({ default: () => null }));

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

beforeEach(() => {
  vi.mocked(commands.preferencesSnapshot).mockImplementation(() =>
    ok({ version: 1, values: values(), scores_stale: false } satisfies PreferencesSnapshot),
  );
  vi.mocked(commands.embeddingsStatus).mockImplementation(() =>
    ok({
      available: false,
      opted_in: false,
      installed: false,
      model_id: "multilingual-e5-small",
      destination: null,
      approximate_bytes: 510_656_512,
    }),
  );
});

function Pathname() {
  return <span data-testid="pathname">{useLocation().pathname}</span>;
}

function renderSettings(initialPath: string) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[initialPath]}>
        <Routes>
          <Route path="settings/*" element={<Settings />} />
        </Routes>
        <Pathname />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

const nav = () => screen.getByRole("navigation", { name: "Settings" });
const settingsLink = (name: string) => {
  const link = screen.getAllByRole("link", { name }).find((node) => nav().contains(node));
  if (!link) throw new Error(`no Settings nav link named ${name}`);
  return link;
};
const pathname = () => screen.getByTestId("pathname").textContent;

describe("Settings nav — absolute links (FX1, issue 3)", () => {
  it("goes from a nested URL to Scoring at /settings/scoring, and renders its page", () => {
    renderSettings("/settings/privacy/audit");

    fireEvent.click(settingsLink("Scoring"));

    expect(pathname()).toBe("/settings/scoring");
    expect(screen.getByRole("heading", { level: 1, name: "Scoring" })).toBeTruthy();
    expect(settingsLink("Scoring").getAttribute("aria-current")).toBe("page");
  });

  it("reaches each page on two clicks in a row and marks only the clicked entry active", () => {
    renderSettings("/settings");

    fireEvent.click(settingsLink("Scoring"));
    expect(pathname()).toBe("/settings/scoring");
    expect(screen.getByRole("heading", { level: 1, name: "Scoring" })).toBeTruthy();

    fireEvent.click(settingsLink("Preferences"));
    expect(pathname()).toBe("/settings/preferences");
    expect(screen.getByRole("heading", { level: 1, name: "Preferences" })).toBeTruthy();
    expect(settingsLink("Preferences").getAttribute("aria-current")).toBe("page");
    expect(settingsLink("Scoring").getAttribute("aria-current")).toBeNull();
  });
});
