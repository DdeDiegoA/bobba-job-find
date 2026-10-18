import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  type CapabilitySnapshot,
  commands,
  type InterviewOutcome,
  type ProfileSnapshot,
  type ProposalView,
  type QuestionView,
} from "@/lib/ipc/bindings";
import "@/lib/i18n";
import { GapsEntry } from "./gaps-entry";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    capabilitiesSnapshot: vi.fn(),
    profileGapsStart: vi.fn(),
    profileGapsAnswer: vi.fn(),
    profileGapsConfirm: vi.fn(),
    profileGapsDiscard: vi.fn(),
    profileSaveFact: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

/** The capabilities the core reports (FX9 AC3): AI on unless the test says no. */
const capabilities = (available = true): CapabilitySnapshot => ({
  limited: !available,
  capabilities: [{ id: "profile_gaps", available, reason: available ? null : "no_agent" }],
});

const question = (over: Partial<QuestionView> = {}): QuestionView => ({
  key: "cefr:en",
  kind: "cefr",
  optional: false,
  text_en: "How well do you speak English?",
  text_es: "¿Qué tan bien hablas inglés?",
  ...over,
});

const proposal = (over: Partial<ProposalView> = {}): ProposalView => ({
  id: "prop_1",
  item_id: null,
  kind: "experience",
  text_en: "Led the migration to Rust.",
  text_es: "Lideré la migración a Rust.",
  evidence: null,
  detail_json: "{}",
  source_excerpt: null,
  base_revision: null,
  untrusted_origin: false,
  created_at: "2026-10-18T09:00:00.000Z",
  ...over,
});

const outcome = (over: Partial<InterviewOutcome> = {}): InterviewOutcome => ({
  run_id: "run_1",
  questions: [question()],
  proposals: [],
  ...over,
});

const snapshot = (): ProfileSnapshot => ({
  profile_version: 3,
  facts: [],
  pending_proposals: 0,
});

function renderEntry() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={["/profile"]}>
        <Routes>
          <Route path="/profile" element={<GapsEntry />} />
          <Route path="/settings/ai-engine" element={<p>Engine settings</p>} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

const clickReview = async () => {
  const button = await screen.findByRole("button", { name: "Review gaps" });
  await waitFor(() => expect(button).toBeEnabled());
  fireEvent.click(button);
};

beforeEach(() => {
  for (const fn of Object.values(commands)) vi.mocked(fn).mockReset();
  vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() => ok(capabilities()));
  vi.mocked(commands.profileGapsStart).mockImplementation(() => ok(outcome()));
  vi.mocked(commands.profileGapsAnswer).mockImplementation(() => ok(snapshot()));
  vi.mocked(commands.profileGapsConfirm).mockImplementation(() => ok(snapshot()));
  vi.mocked(commands.profileGapsDiscard).mockImplementation(() => ok(null));
});

describe("Profile gaps entry — re-run the interview (FX9 AC1)", () => {
  it("runs profileGapsStart and shows the questions it returns", async () => {
    renderEntry();
    await clickReview();

    await waitFor(() =>
      expect(commands.profileGapsStart).toHaveBeenCalledWith(null, expect.any(String)),
    );
    expect(await screen.findByText("How well do you speak English?")).toBeTruthy();
  });

  it("renders the run's proposals through the shared review card, verbatim", async () => {
    const long = "Led the migration to Rust.\nCut CI time by 40%.";
    vi.mocked(commands.profileGapsStart).mockImplementation(() =>
      ok(outcome({ proposals: [proposal({ text_en: long })] })),
    );
    renderEntry();
    await clickReview();

    const [text] = await screen.findAllByText(long, { normalizer: (value) => value });
    expect(text.tagName).toBe("P");
    expect(text.className).toContain("whitespace-pre-wrap");
    expect(screen.getByRole("button", { name: "Confirm" })).toBeTruthy();
  });

  it("says so when the profile leaves no gaps", async () => {
    vi.mocked(commands.profileGapsStart).mockImplementation(() =>
      ok(outcome({ questions: [], proposals: [] })),
    );
    renderEntry();
    await clickReview();

    expect(await screen.findByText(/Nothing left to ask/)).toBeTruthy();
  });
});

describe("Profile gaps entry — per-item approval (FX9 AC2)", () => {
  it("saves only the proposal the user confirms", async () => {
    vi.mocked(commands.profileGapsStart).mockImplementation(() =>
      ok(
        outcome({
          questions: [],
          proposals: [
            proposal({ id: "prop_1", text_en: "First suggested fact." }),
            proposal({ id: "prop_2", text_en: "Second suggested fact." }),
          ],
        }),
      ),
    );
    renderEntry();
    await clickReview();

    const [first] = await screen.findAllByRole("button", { name: "Confirm" });
    fireEvent.click(first);

    await waitFor(() => expect(commands.profileGapsConfirm).toHaveBeenCalledWith("prop_1", null));
    // The other proposal stays pending, and nothing is written behind the user's back.
    expect(await screen.findByText("Second suggested fact.")).toBeTruthy();
    expect(screen.getAllByRole("button", { name: "Confirm" })).toHaveLength(1);
    expect(commands.profileSaveFact).not.toHaveBeenCalled();
  });

  it("saves only the answer the user gives", async () => {
    renderEntry();
    await clickReview();
    await screen.findByText("How well do you speak English?");

    fireEvent.change(screen.getByLabelText("Level (CEFR)"), { target: { value: "C1" } });
    fireEvent.click(screen.getByRole("button", { name: "Save answer" }));

    await waitFor(() =>
      expect(commands.profileGapsAnswer).toHaveBeenCalledWith(
        "cefr:en",
        { kind: "cefr", level: "C1" },
        expect.any(String),
      ),
    );
    expect(commands.profileGapsConfirm).not.toHaveBeenCalled();
    expect(commands.profileSaveFact).not.toHaveBeenCalled();
  });

  it("discards a proposal without saving it", async () => {
    vi.mocked(commands.profileGapsStart).mockImplementation(() =>
      ok(outcome({ questions: [], proposals: [proposal()] })),
    );
    renderEntry();
    await clickReview();

    fireEvent.click(await screen.findByRole("button", { name: "Delete" }));

    await waitFor(() => expect(commands.profileGapsDiscard).toHaveBeenCalledWith("prop_1"));
    expect(commands.profileGapsConfirm).not.toHaveBeenCalled();
    expect(commands.profileSaveFact).not.toHaveBeenCalled();
  });
});

describe("Profile gaps entry — Limited mode (FX9 AC3)", () => {
  it("disables the button, explains why, and links to Settings > AI engine", async () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() => ok(capabilities(false)));
    renderEntry();

    expect(await screen.findByText("AI features are off — no Agent CLI connected.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Review gaps" })).toBeDisabled();
    expect(screen.getByRole("link", { name: "Open Settings > AI engine" })).toBeTruthy();
  });

  it("navigates from Limited mode to Settings > AI engine", async () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() => ok(capabilities(false)));
    renderEntry();

    fireEvent.click(await screen.findByRole("link", { name: "Open Settings > AI engine" }));
    expect(await screen.findByText("Engine settings")).toBeTruthy();
  });
});

describe("Profile gaps entry — run feedback", () => {
  it("announces the run while it is writing the questions", async () => {
    let settle!: (value: { status: "ok"; data: InterviewOutcome }) => void;
    vi.mocked(commands.profileGapsStart).mockImplementation(
      () =>
        new Promise((resolve) => {
          settle = resolve;
        }),
    );
    renderEntry();
    await clickReview();

    expect(await screen.findByRole("status")).toHaveTextContent("Boba is writing your questions");
    settle({ status: "ok", data: outcome() });
  });

  it("announces a failed run as an alert and saves nothing", async () => {
    vi.mocked(commands.profileGapsStart).mockImplementation(() =>
      Promise.reject({
        code: "agent_unsupported",
        message_key: "errors.agent.unsupported",
        details: null,
      }),
    );
    renderEntry();
    await clickReview();

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/needs an AI tool with a verified tool lockdown/);
    expect(commands.profileGapsConfirm).not.toHaveBeenCalled();
    expect(commands.profileSaveFact).not.toHaveBeenCalled();
  });
});
