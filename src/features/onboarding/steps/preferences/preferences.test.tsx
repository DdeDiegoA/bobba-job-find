import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type Preferences, type PreferencesSnapshot } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import PreferencesStep from "./index";

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
function fakeCore() {
  stored = { version: 0, values: values(), scores_stale: false };
  vi.mocked(commands.preferencesSnapshot).mockImplementation(() => ok(stored));
  vi.mocked(commands.preferencesSave).mockImplementation((input) => {
    if (JSON.stringify(input) !== JSON.stringify(stored.values)) {
      stored = { version: stored.version + 1, values: input, scores_stale: true };
    }
    return ok(stored);
  });
}

const lastSaved = (): Preferences =>
  vi.mocked(commands.preferencesSave).mock.calls.at(-1)?.[0] as Preferences;

function renderStep() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <PreferencesStep answers={{}} setAnswer={() => undefined} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.preferencesSnapshot).mockReset();
  vi.mocked(commands.preferencesSave).mockReset();
  fakeCore();
});

describe("Preferences step — what I am looking for (AC1, FR-7)", () => {
  it("offers every preference FR-7 asks for", async () => {
    renderStep();
    for (const label of [
      "Roles and titles",
      "Seniority",
      "Salary floor",
      "Locations",
      "Timezone window",
      "Work mode",
      "Working languages",
      "Deal-breakers",
      "Excluded companies",
    ]) {
      expect(await screen.findByText(label), label).toBeTruthy();
    }
  });

  it("saves target roles (multiple) as they are added", async () => {
    renderStep();
    await addTo("Roles and titles", "Backend Engineer");
    await addTo("Roles and titles", "Platform Engineer");

    await waitFor(() => expect(commands.preferencesSave).toHaveBeenCalledTimes(2));
    expect(lastSaved().roles).toEqual(["Backend Engineer", "Platform Engineer"]);
  });

  it("saves the salary floor as amount_minor, currency, period and firmness (AD-17)", async () => {
    renderStep();
    const amount = await screen.findByLabelText("Amount");
    fireEvent.change(amount, { target: { value: "60000" } });
    fireEvent.blur(amount);
    await waitFor(() => expect(commands.preferencesSave).toHaveBeenCalledTimes(1));
    expect(lastSaved().salary_floor).toEqual({
      amount_minor: 6_000_000,
      currency: "USD",
      period: "year",
      firmness: "hard",
    });

    fireEvent.change(screen.getByLabelText("Period"), { target: { value: "month" } });
    fireEvent.click(screen.getByLabelText("Soft — only lower the score"));

    await waitFor(() =>
      expect(lastSaved().salary_floor).toEqual({
        amount_minor: 6_000_000,
        currency: "USD",
        period: "month",
        firmness: "soft",
      }),
    );
  });

  it("saves seniority, work modes and working languages", async () => {
    renderStep();
    fireEvent.change(await screen.findByLabelText("Minimum"), { target: { value: "senior" } });
    fireEvent.click(screen.getByLabelText("Hybrid"));
    fireEvent.click(screen.getByLabelText("Español"));

    await waitFor(() =>
      expect(lastSaved()).toMatchObject({
        seniority_min: "senior",
        work_modes: { remote: true, hybrid: true, onsite: false },
        working_languages: ["en", "es"],
      }),
    );
  });

  it("saves locations, deal-breakers and excluded companies", async () => {
    renderStep();
    await addTo("Locations", "Berlin");
    await addTo("Deal-breaker keywords", "on-call");
    fireEvent.change(screen.getByLabelText("Anything else"), { target: { value: "No crypto." } });
    await addTo("Excluded companies", "Acme Corp");

    await waitFor(() =>
      expect(lastSaved()).toMatchObject({
        locations: ["Berlin"],
        deal_breaker_keywords: ["on-call"],
        deal_breaker_notes: "No crypto.",
        excluded_companies: ["Acme Corp"],
      }),
    );
  });

  it("shows the core's error in plain language when saving fails", async () => {
    renderStep();
    vi.mocked(commands.preferencesSave).mockImplementation(() =>
      Promise.resolve({
        status: "error" as const,
        error: { code: "internal" as const, message_key: "errors.preferences.store" },
      }),
    );

    await addTo("Roles and titles", "SRE");

    expect(await screen.findByText("Your preferences could not be read or saved.")).toBeTruthy();
  });
});

async function addTo(label: string, value: string) {
  const input = await screen.findByLabelText(label);
  fireEvent.change(input, { target: { value } });
  fireEvent.click(screen.getByRole("button", { name: `Add ${label}` }));
}
