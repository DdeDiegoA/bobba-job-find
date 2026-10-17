import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  commands,
  type EngineCard,
  type EngineSnapshot,
  type TestCallResult,
} from "@/lib/ipc/bindings";
import "@/lib/i18n";
import EngineStep, { engineStepIsValid } from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    agentEngineSnapshot: vi.fn(),
    agentEngineChoose: vi.fn(),
    agentTestCall: vi.fn(),
    agentModels: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const card = (over: Partial<EngineCard>): EngineCard => ({
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

const snapshot = (engines: EngineCard[], chosen: EngineSnapshot["chosen"] = null) => ({
  engines,
  chosen,
});

const passed = (over: Partial<TestCallResult> = {}): TestCallResult => ({
  ok: true,
  cli: "claude_code",
  model: "sonnet",
  latency_ms: 812,
  reply: "pong",
  failure_reason: null,
  message_key: null,
  tokens_in: null,
  tokens_out: null,
  cost_usd: null,
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

/** Stands in for the wizard: keeps answers and shows them, like `StepProps`. */
function Harness({ draft = {} }: { draft?: Record<string, unknown> }) {
  const [answers, setAnswers] = useState(draft);
  return (
    <>
      <EngineStep
        answers={answers}
        setAnswer={(key, value) => setAnswers((a) => ({ ...a, [key]: value }))}
      />
      <output data-testid="answers">{JSON.stringify(answers)}</output>
    </>
  );
}

const answers = (): Record<string, unknown> =>
  JSON.parse(screen.getByTestId("answers").textContent ?? "{}") as Record<string, unknown>;

function renderStep(draft: Record<string, unknown> = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Harness draft={draft} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.agentEngineSnapshot).mockReset();
  vi.mocked(commands.agentEngineChoose).mockReset();
  vi.mocked(commands.agentTestCall).mockReset();
  vi.mocked(commands.agentModels).mockReset();
  vi.mocked(commands.agentEngineSnapshot).mockImplementation(() => {
    return ok(snapshot([card({})]));
  });
  vi.mocked(commands.agentModels).mockImplementation(() => ok([]));
  vi.mocked(commands.agentEngineChoose).mockImplementation((cli, model) => ok({ cli, model }));
  vi.mocked(commands.agentTestCall).mockImplementation(() => ok(passed()));
});

describe("Engine step — pick a CLI and model (FR-2, FR-3, AC1)", () => {
  it("lists every CLI the probe found, and what is missing with an install link", async () => {
    vi.mocked(commands.agentEngineSnapshot).mockImplementation(() =>
      ok(
        snapshot([
          card({}),
          card({
            cli: "codex",
            name: "Codex CLI",
            installed: false,
            version: "",
            path: null,
            health: "unhealthy",
            install_url: "https://example.test/codex",
          }),
        ]),
      ),
    );
    renderStep();

    expect(await screen.findByText("Claude Code")).toBeTruthy();
    expect(screen.getByText(/Version 2.1.301/)).toBeTruthy();
    expect(screen.getByText("Codex CLI")).toBeTruthy();
    expect(screen.getByText("Not installed")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Get Codex CLI" }));
  });

  it("says which tasks stay unavailable on a CLI without a verified lockdown (AC2)", async () => {
    vi.mocked(commands.agentEngineSnapshot).mockImplementation(() =>
      ok(
        snapshot([
          card({
            cli: "aider",
            name: "Aider",
            path: "/usr/local/bin/aider",
            unsupported_tasks: ["extract_postings", "classify_emails", "tailor_application"],
          }),
        ]),
      ),
    );
    renderStep();

    expect(await screen.findByText("Aider")).toBeTruthy();
    const notice = screen.getByText(/stay unavailable/);
    expect(notice.textContent).toContain("Reading postings");
    expect(notice.textContent).toContain("Classifying email");
    expect(notice.textContent).toContain("Tailoring a CV or cover letter");
    // The step never offers an override (ADR-010 §2).
    expect(screen.queryByText(/run anyway/i)).toBeNull();
  });

  it("tells the user where Profile and Offer text goes before they continue (NFR-3)", async () => {
    renderStep();

    expect(await screen.findByText("Where your data goes")).toBeTruthy();
    const notice = screen.getByText(/sends Profile and Offer text/);
    expect(notice.textContent).toContain("Claude Code");
    expect(notice.textContent).toContain("Anthropic");
    expect(notice.textContent).toContain("local model");

    const ack = screen.getByRole("checkbox", {
      name: "I understand where my data goes",
    }) as HTMLInputElement;
    expect(ack.checked).toBe(false);
    expect(engineStepIsValid(answers())).toBe(false);
    fireEvent.click(ack);
    expect(answers().disclosureAck).toBe(true);
  });

  it("runs the test call and shows latency and the model identity (AC1)", async () => {
    renderStep();
    fireEvent.change(await screen.findByLabelText("Model"), {
      target: { value: "sonnet" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));

    expect(await screen.findByText("Answered in 812 ms as sonnet.")).toBeTruthy();
    expect(vi.mocked(commands.agentTestCall)).toHaveBeenCalledWith("claude_code", "sonnet");
    // A passing test is the choice Settings edits later (FR-17).
    await waitFor(() =>
      expect(vi.mocked(commands.agentEngineChoose)).toHaveBeenCalledWith("claude_code", "sonnet"),
    );
    expect(answers().testedOk).toBe(true);
  });

  it("shows the CLI's own error and a fix hint when the test fails (AC1)", async () => {
    vi.mocked(commands.agentTestCall).mockImplementation(() => ok(failed()));
    renderStep();
    fireEvent.click(await screen.findByRole("button", { name: "Test connection" }));

    expect(await screen.findByText("The connection test failed.")).toBeTruthy();
    // The reason is translated from the run's message key, never raw text.
    expect(screen.getByText(/sign in again/)).toBeTruthy();
    // The CLI's own words and what to do about them.
    // (The harness's answers dump repeats the text; the step's own copy is one of them.)
    expect(screen.getAllByText(/error: not logged in/).length).toBeGreaterThan(0);
    expect(screen.getByText(/log in, then test again/)).toBeTruthy();
    expect(answers().testedOk).toBe(false);
  });

  it("drops a passing result when the CLI or the model changes", async () => {
    vi.mocked(commands.agentEngineSnapshot).mockImplementation(() =>
      ok(
        snapshot([
          card({}),
          card({ cli: "codex", name: "Codex CLI", path: "/usr/local/bin/codex" }),
        ]),
      ),
    );
    renderStep();
    fireEvent.click(await screen.findByRole("button", { name: "Test connection" }));
    expect(await screen.findByText("Answered in 812 ms as sonnet.")).toBeTruthy();

    fireEvent.change(screen.getByLabelText("Model"), { target: { value: "opus" } });
    expect(screen.queryByText("Answered in 812 ms as sonnet.")).toBeNull();
    expect(answers().testedOk).toBe(false);

    fireEvent.click(screen.getByRole("radio", { name: /Codex CLI/ }));
    expect(answers().cli).toBe("codex");
  });

  it("resumes with the stored choice and a previous pass", async () => {
    vi.mocked(commands.agentEngineSnapshot).mockImplementation(() =>
      ok(snapshot([card({})], { cli: "claude_code", model: "sonnet" })),
    );
    renderStep({
      cli: "claude_code",
      model: "sonnet",
      testedOk: true,
      lastResult: passed(),
      disclosureAck: true,
    });

    expect(await screen.findByText("Answered in 812 ms as sonnet.")).toBeTruthy();
    expect((screen.getByLabelText("Model") as HTMLInputElement).value).toBe("sonnet");
    expect(engineStepIsValid(answers())).toBe(true);
  });

  it("cannot continue without both a passing test and the acknowledgement", () => {
    expect(engineStepIsValid({ testedOk: true })).toBe(false);
    expect(engineStepIsValid({ disclosureAck: true })).toBe(false);
    expect(engineStepIsValid({ testedOk: true, disclosureAck: true })).toBe(true);
  });
});
