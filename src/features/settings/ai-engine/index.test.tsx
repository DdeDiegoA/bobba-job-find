import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { type BudgetSnapshot, commands, type TestCallResult } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SettingsAiEngine from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    agentBudgetSnapshot: vi.fn(),
    agentBudgetSave: vi.fn(),
    agentEngineSnapshot: vi.fn(),
    agentTestCall: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const snapshot = (over: Partial<BudgetSnapshot> = {}): BudgetSnapshot => ({
  offers_per_run: 30,
  max_offers_per_run: 500,
  limits: [
    { task: "extract_postings", max_turns: 30, max_tool_calls: 60, wall_clock_secs: 600 },
    { task: "tailor_application", max_turns: 40, max_tool_calls: 60, wall_clock_secs: 900 },
  ],
  ...over,
});

const usage = (over: Partial<TestCallResult> = {}): TestCallResult => ({
  ok: true,
  cli: "claude_code",
  model: "sonnet",
  latency_ms: 812,
  reply: "pong",
  failure_reason: null,
  message_key: null,
  tokens_in: 120,
  tokens_out: 40,
  cost_usd: 0.002,
  ...over,
});

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SettingsAiEngine />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.agentBudgetSnapshot).mockReset();
  vi.mocked(commands.agentBudgetSave).mockReset();
  vi.mocked(commands.agentEngineSnapshot).mockReset();
  vi.mocked(commands.agentTestCall).mockReset();
  vi.mocked(commands.agentBudgetSnapshot).mockImplementation(() => ok(snapshot()));
  vi.mocked(commands.agentBudgetSave).mockImplementation((value) => ok(value));
  vi.mocked(commands.agentEngineSnapshot).mockImplementation(() =>
    ok({
      engines: [],
      chosen: { cli: "claude_code", model: "sonnet" },
    }),
  );
  vi.mocked(commands.agentTestCall).mockImplementation(() => ok(usage()));
});

describe("Settings → AI engine (S2.5)", () => {
  it("shows the stored per-run budget and saves a new one", async () => {
    renderPage();

    const input = await screen.findByLabelText("Offers analysed per run");
    expect((input as HTMLInputElement).value).toBe("30");

    fireEvent.change(input, { target: { value: "12" } });
    fireEvent.click(screen.getByRole("button", { name: "Save budget" }));

    await waitFor(() => expect(vi.mocked(commands.agentBudgetSave)).toHaveBeenCalledWith(12));
    expect(await screen.findByText("Budget saved.")).toBeTruthy();
  });

  it("does not save a value outside the core's range", async () => {
    renderPage();

    const input = await screen.findByLabelText("Offers analysed per run");
    fireEvent.change(input, { target: { value: "0" } });

    expect(screen.getByRole("button", { name: "Save budget" })).toHaveProperty("disabled", true);
  });

  it("lists every per-task limit the core enforces", async () => {
    renderPage();

    expect(await screen.findByText("Reading postings")).toBeTruthy();
    expect(screen.getByText("Tailoring a CV or cover letter")).toBeTruthy();
    expect(screen.getByText("30 turns · 60 tool calls · 600 s")).toBeTruthy();
    expect(screen.getByText("40 turns · 60 tool calls · 900 s")).toBeTruthy();
  });

  it("shows the token usage the CLI reported", async () => {
    renderPage();

    fireEvent.click(await screen.findByRole("button", { name: "Run a test call" }));

    expect(await screen.findByText("120")).toBeTruthy();
    expect(screen.getByText("40")).toBeTruthy();
    expect(screen.getByText("$0.0020")).toBeTruthy();
  });

  it("says usage was not reported instead of showing zeros", async () => {
    vi.mocked(commands.agentTestCall).mockImplementation(() =>
      ok(usage({ tokens_in: null, tokens_out: null, cost_usd: null })),
    );
    renderPage();

    fireEvent.click(await screen.findByRole("button", { name: "Run a test call" }));

    expect(await screen.findByText("Not reported by this CLI.")).toBeTruthy();
  });
});
