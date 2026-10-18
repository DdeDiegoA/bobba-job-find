import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useCapabilities } from "@/lib/capabilities";
import {
  type BudgetSnapshot,
  commands,
  type EngineCard,
  type EngineSnapshot,
  type TestCallResult,
} from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SettingsAiEngine from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    agentBudgetSnapshot: vi.fn(),
    agentBudgetSave: vi.fn(),
    agentEngineSnapshot: vi.fn(),
    agentEngineChoose: vi.fn(),
    agentModels: vi.fn(),
    agentTestCall: vi.fn(),
    capabilitiesSnapshot: vi.fn(),
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

const card = (over: Partial<EngineCard> = {}): EngineCard => ({
  cli: "claude_code",
  name: "Claude Code",
  installed: true,
  version: "2.1.301",
  path: "/usr/local/bin/claude",
  health: "healthy",
  install_url: "https://example.test/claude",
  unsupported_tasks: [],
  ...over,
});

const failed = (over: Partial<TestCallResult> = {}): TestCallResult => ({
  ok: false,
  cli: "claude_code",
  model: null,
  latency_ms: 40,
  reply: "error: not logged in",
  failure_reason: "auth_expired",
  message_key: "errors.agent.auth_expired",
  tokens_in: null,
  tokens_out: null,
  cost_usd: null,
  ...over,
});

/** Reads the session's capability gate, so a saved engine can be seen leaving Limited mode. */
function LimitedProbe() {
  const capabilities = useCapabilities();
  const state =
    capabilities.data === undefined ? "loading" : capabilities.data.limited ? "limited" : "ready";
  return <output data-testid="limited">{state}</output>;
}

function renderPage({ probe = false }: { probe?: boolean } = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SettingsAiEngine />
      {probe ? <LimitedProbe /> : null}
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.agentBudgetSnapshot).mockReset();
  vi.mocked(commands.agentBudgetSave).mockReset();
  vi.mocked(commands.agentEngineSnapshot).mockReset();
  vi.mocked(commands.agentEngineChoose).mockReset();
  vi.mocked(commands.agentModels).mockReset();
  vi.mocked(commands.agentTestCall).mockReset();
  vi.mocked(commands.capabilitiesSnapshot).mockReset();
  vi.mocked(commands.agentBudgetSnapshot).mockImplementation(() => ok(snapshot()));
  vi.mocked(commands.agentBudgetSave).mockImplementation((value) => ok(value));
  vi.mocked(commands.agentEngineSnapshot).mockImplementation(() =>
    ok({
      engines: [],
      chosen: { cli: "claude_code", model: "sonnet" },
    }),
  );
  vi.mocked(commands.agentEngineChoose).mockImplementation((cli, model) => ok({ cli, model }));
  vi.mocked(commands.agentModels).mockImplementation(() => ok([]));
  vi.mocked(commands.agentTestCall).mockImplementation(() => ok(usage()));
  vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() =>
    ok({
      limited: vi.mocked(commands.agentEngineChoose).mock.calls.length === 0,
      capabilities: [],
    }),
  );
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

/** FX7: the onboarding engine step's picker, reused here to switch engines. */
describe("Settings → AI engine: engine picker (FX7)", () => {
  /** A probe with the picked CLI installed and nothing stored yet. */
  function withInstalledEngines() {
    let chosen: EngineSnapshot["chosen"] = null;
    vi.mocked(commands.agentEngineSnapshot).mockImplementation(() =>
      ok({ engines: [card()], chosen }),
    );
    vi.mocked(commands.agentEngineChoose).mockImplementation((cli, model) => {
      chosen = { cli, model };
      return ok(chosen);
    });
    return { chosen: () => chosen };
  }

  it("shows the installed CLIs, a model field, Test and Save (AC1)", async () => {
    withInstalledEngines();
    renderPage();

    expect(await screen.findByText("Claude Code")).toBeTruthy();
    expect(screen.getByLabelText("Model")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Test connection" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Save engine" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Save engine" })).toHaveProperty("disabled", true);
  });

  it("saves only after a passing test, then refreshes the snapshot and usage (AC2)", async () => {
    const stored = withInstalledEngines();
    renderPage();

    fireEvent.change(await screen.findByLabelText("Model"), { target: { value: "sonnet" } });
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));

    expect(await screen.findByText("Answered in 812 ms as sonnet.")).toBeTruthy();
    // The pass alone does not choose anything: Save is the one write.
    expect(vi.mocked(commands.agentEngineChoose)).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Save engine" })).toHaveProperty("disabled", false);

    const snapshotCalls = vi.mocked(commands.agentEngineSnapshot).mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: "Save engine" }));

    await waitFor(() =>
      expect(vi.mocked(commands.agentEngineChoose)).toHaveBeenCalledWith("claude_code", "sonnet"),
    );
    expect(stored.chosen()).toEqual({ cli: "claude_code", model: "sonnet" });
    expect(await screen.findByText("Engine saved.")).toBeTruthy();
    // The engine snapshot is refreshed, so Usage now offers its test call.
    await waitFor(() =>
      expect(vi.mocked(commands.agentEngineSnapshot).mock.calls.length).toBeGreaterThan(
        snapshotCalls,
      ),
    );
    expect(await screen.findByRole("button", { name: "Run a test call" })).toBeTruthy();
  });

  it("disables Save and shows the translated error when the test fails (AC3)", async () => {
    withInstalledEngines();
    vi.mocked(commands.agentTestCall).mockImplementation(() => ok(failed()));
    renderPage();

    fireEvent.click(await screen.findByRole("button", { name: "Test connection" }));

    expect(await screen.findByText("The connection test with Claude Code failed.")).toBeTruthy();
    expect(screen.getByText(/sign in again/)).toBeTruthy();
    const save = screen.getByRole("button", { name: "Save engine" });
    expect(save).toHaveProperty("disabled", true);
    fireEvent.click(save);
    expect(vi.mocked(commands.agentEngineChoose)).not.toHaveBeenCalled();
  });

  it("leaves Limited mode once an engine is saved (AC4)", async () => {
    withInstalledEngines();
    renderPage({ probe: true });

    await waitFor(() =>
      expect((screen.getByTestId("limited") as HTMLElement).textContent).toBe("limited"),
    );
    fireEvent.click(await screen.findByRole("button", { name: "Test connection" }));
    expect(await screen.findByText("Answered in 812 ms as sonnet.")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Save engine" }));

    await waitFor(() => expect(vi.mocked(commands.agentEngineChoose)).toHaveBeenCalled());
    await waitFor(() =>
      expect((screen.getByTestId("limited") as HTMLElement).textContent).toBe("ready"),
    );
  });
});
