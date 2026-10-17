import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  type BoardView,
  commands,
  type DecisionView,
  type RuleInput,
  type RulesSnapshot,
} from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SettingsApplyingRules from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    applyRulesSnapshot: vi.fn(),
    applyRuleSave: vi.fn(),
    applyRuleDelete: vi.fn(),
    applyRulesSetEnabled: vi.fn(),
    applyRulesReset: vi.fn(),
    applyRulesSetGlobalCap: vi.fn(),
    applyRulesExplain: vi.fn(),
    trackerBoard: vi.fn(),
    sessionAutoSnapshot: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const rule = (over: Partial<RulesSnapshot["rules"][number]> = {}) => ({
  id: "rule_1",
  name: "Email × remote",
  enabled: true,
  source_ids: ["remotive"],
  channels: ["email"],
  min_rank: 80,
  min_fit: 80,
  min_confidence: "medium",
  allow_flags: false,
  min_verdict: "good",
  daily_cap: 5,
  per_company_cap: 1,
  per_company_days: 30,
  active_start_minute: 540,
  active_end_minute: 1080,
  created_at: "2026-10-17T00:00:00Z",
  updated_at: "2026-10-17T00:00:00Z",
  ...over,
});

const snapshot = (over: Partial<RulesSnapshot> = {}): RulesSnapshot => ({
  auto_enabled: false,
  auto_confirmed: false,
  global_daily_cap: 10,
  rules: [rule()],
  ...over,
});

const board: BoardView = {
  columns: [
    {
      stage: "saved",
      status: "saved",
      terminal: false,
      count: 1,
      cards: [
        {
          id: "app_1",
          status: "saved",
          stage: "saved",
          title: "Data Eng",
          company: "Tapi Tea",
          next_action: null,
        },
      ],
    },
  ],
};

let stored: RulesSnapshot;

beforeEach(() => {
  vi.mocked(commands.applyRulesSnapshot).mockReset();
  vi.mocked(commands.applyRuleSave).mockReset();
  vi.mocked(commands.applyRuleDelete).mockReset();
  vi.mocked(commands.applyRulesSetEnabled).mockReset();
  vi.mocked(commands.applyRulesReset).mockReset();
  vi.mocked(commands.applyRulesSetGlobalCap).mockReset();
  vi.mocked(commands.applyRulesExplain).mockReset();
  vi.mocked(commands.trackerBoard).mockReset();
  vi.mocked(commands.sessionAutoSnapshot).mockReset();
  vi.mocked(commands.sessionAutoSnapshot).mockImplementation(() => ok([]));

  stored = snapshot();
  vi.mocked(commands.applyRulesSnapshot).mockImplementation(() => ok(stored));
  vi.mocked(commands.trackerBoard).mockImplementation(() => ok(board));
  vi.mocked(commands.applyRulesSetEnabled).mockImplementation((enabled, confirmed) => {
    stored = {
      ...stored,
      auto_enabled: enabled && confirmed,
      auto_confirmed: stored.auto_confirmed || (enabled && confirmed),
    };
    return ok(stored);
  });
});

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SettingsApplyingRules />
    </QueryClientProvider>,
  );
}

describe("Settings — Apply (S9.6, EXPERIENCE §9.12)", () => {
  it("keeps Auto off until the consequences dialog is confirmed (AC3)", async () => {
    renderPage();
    fireEvent.click(await screen.findByRole("button", { name: "Enable Auto" }));

    const confirm = screen.getByRole("button", { name: "Turn Auto on" }) as HTMLButtonElement;
    expect(confirm.disabled).toBe(true);

    fireEvent.click(screen.getByRole("checkbox", { name: /I understand/ }));
    expect(confirm.disabled).toBe(false);
    fireEvent.click(confirm);

    await waitFor(() => expect(commands.applyRulesSetEnabled).toHaveBeenCalledWith(true, true));
  });

  it("does not call the core when the dialog is dismissed", async () => {
    renderPage();
    fireEvent.click(await screen.findByRole("button", { name: "Enable Auto" }));
    fireEvent.click(screen.getByRole("button", { name: "Keep Auto off" }));

    expect(commands.applyRulesSetEnabled).not.toHaveBeenCalled();
    expect(
      screen.getByText("Auto is off. Boba prepares applications; you send them."),
    ).toBeTruthy();
  });

  it("saves a new rule with the defaults the form offers (AC1)", async () => {
    let saved: RuleInput | null = null;
    vi.mocked(commands.applyRuleSave).mockImplementation((input) => {
      saved = input;
      return ok(rule());
    });

    renderPage();
    fireEvent.click(await screen.findByRole("button", { name: "New rule" }));
    fireEvent.change(screen.getByLabelText("Rule name"), { target: { value: "Email × remote" } });
    fireEvent.click(screen.getByRole("button", { name: "Save rule" }));

    await waitFor(() => expect(saved).not.toBeNull());
    const input = saved as unknown as RuleInput;
    expect(input.id).toBeNull();
    expect(input.channels).toEqual(["email"]);
    expect(input.min_rank).toBe(80);
    expect(input.min_fit).toBe(80);
    expect(input.min_confidence).toBe("medium");
    expect(input.allow_flags).toBe(false);
    expect(input.daily_cap).toBe(5);
    expect(input.per_company_cap).toBe(1);
    expect(input.per_company_days).toBe(30);
    expect(input.active_start_minute).toBe(540);
    expect(input.active_end_minute).toBe(1080);
  });

  it("never lets a new rule pick the session board (AC2)", async () => {
    renderPage();
    fireEvent.click(await screen.findByRole("button", { name: "New rule" }));
    const session = screen.getByRole("checkbox", { name: "Session Board" }) as HTMLInputElement;
    expect(session.disabled).toBe(true);
  });

  it("shows the decision reasons verbatim (AC2)", async () => {
    const decision: DecisionView = {
      kind: "review",
      rule_id: null,
      reasons: [
        { code: "rank_below", params: { found: "61", min: "80" } },
        { code: "outside_active_hours", params: { start_minute: "540", end_minute: "1080" } },
      ],
    };
    vi.mocked(commands.applyRulesExplain).mockImplementation(() => ok(decision));

    renderPage();
    fireEvent.change(await screen.findByLabelText("Application"), {
      target: { value: "app_1" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Check" }));

    expect(await screen.findByText("A human keeps this one. Reasons:")).toBeTruthy();
    expect(screen.getByText("Rank 61 is below the rule's 80.")).toBeTruthy();
    expect(screen.getByText("Outside the rule's active hours (09:00–18:00).")).toBeTruthy();
  });

  it("resets Apply to defaults (AC4, FR-108)", async () => {
    vi.mocked(commands.applyRulesReset).mockImplementation(() => {
      stored = snapshot({ auto_enabled: false, auto_confirmed: false });
      return ok(stored);
    });

    renderPage();
    fireEvent.click(await screen.findByRole("button", { name: "Reset Apply to defaults" }));
    fireEvent.click(screen.getByRole("button", { name: "Reset" }));

    await waitFor(() => expect(commands.applyRulesReset).toHaveBeenCalled());
  });
});
