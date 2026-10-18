import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  type CapabilitySnapshot,
  commands,
  type FactView,
  type Gap,
  type InterviewOutcome,
  type ProfileSnapshot,
  type ProposalView,
  type QuestionView,
  type TranslateOutcome,
} from "@/lib/ipc/bindings";
import "@/lib/i18n";
import GapsStep from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    capabilitiesSnapshot: vi.fn(),
    profileGapsQuestions: vi.fn(),
    profileGapsStart: vi.fn(),
    profileGapsAnswer: vi.fn(),
    profileGapsSkip: vi.fn(),
    profileGapsTranslate: vi.fn(),
    profileGapsConfirm: vi.fn(),
    profileGapsDiscard: vi.fn(),
    profileSnapshot: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const gap = (over: Partial<Gap> = {}): Gap => ({
  kind: "eligibility",
  subject: null,
  item_id: null,
  ...over,
});

const proposal = (over: Partial<ProposalView> = {}): ProposalView => ({
  id: "prop_1",
  item_id: null,
  kind: "eligibility",
  text_en: "Notice period: 1 month",
  text_es: "Preaviso: 1 mes",
  evidence: null,
  detail_json: "{}",
  source_excerpt: null,
  base_revision: null,
  untrusted_origin: false,
  created_at: "2026-10-17T09:00:00.000Z",
  ...over,
});

const outcome = (questions: QuestionView[], proposals: ProposalView[] = []): InterviewOutcome => ({
  run_id: "run_1",
  questions,
  proposals,
});

const translated = (proposals: ProposalView[]): TranslateOutcome => ({
  run_id: "run_2",
  proposals,
});

const snapshot = (facts: FactView[] = []): ProfileSnapshot => ({
  profile_version: 1,
  facts,
  pending_proposals: 0,
});

const fact = (over: Partial<FactView> = {}): FactView => ({
  item_id: "item_1",
  revision_id: "rev_1",
  kind: "notice_period",
  text_en: "Notice period: 1 month",
  text_es: null,
  evidence: null,
  detail_json: "{}",
  missing_variant: "es",
  profile_version: 1,
  provenance: "user",
  from_agent_run: false,
  ...over,
});

/** The capabilities the wizard's engine step would report (S3.3): AI on. */
const capabilities = (available = true): CapabilitySnapshot => ({
  limited: !available,
  capabilities: [
    { id: "profile_gaps", available, reason: available ? null : "no_agent" },
    { id: "profile_translation", available, reason: available ? null : "no_agent" },
  ],
});

function renderStep() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  function Harness() {
    const [answers, setAnswers] = useState<Record<string, unknown>>({});
    return (
      <>
        <GapsStep
          answers={answers}
          setAnswer={(key, value) => setAnswers((a) => ({ ...a, [key]: value }))}
        />
        <output data-testid="answers">{JSON.stringify(answers)}</output>
      </>
    );
  }
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={["/onboarding"]}>
        <Routes>
          <Route path="/onboarding" element={<Harness />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  for (const fn of Object.values(commands)) vi.mocked(fn).mockReset();
  vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() => ok(capabilities()));
  vi.mocked(commands.profileGapsQuestions).mockImplementation(() => ok([gap()]));
  vi.mocked(commands.profileSnapshot).mockImplementation(() => ok(snapshot()));
  vi.mocked(commands.profileGapsStart).mockImplementation(() => ok(outcome([])));
  vi.mocked(commands.profileGapsAnswer).mockImplementation(() => ok(snapshot()));
  vi.mocked(commands.profileGapsSkip).mockImplementation(() => ok(null));
  vi.mocked(commands.profileGapsTranslate).mockImplementation(() => ok(translated([])));
  vi.mocked(commands.profileGapsConfirm).mockImplementation(() => ok(snapshot()));
  vi.mocked(commands.profileGapsDiscard).mockImplementation(() => ok(null));
});

describe("profile gap interview (FR-6, S3.6)", () => {
  it("shows the missing fields with the step's own wording before any run", async () => {
    renderStep();
    expect(await screen.findByText("Where can you legally work without a new visa?")).toBeTruthy();
    expect(commands.profileGapsStart).not.toHaveBeenCalled();
  });

  // AC1: the agent writes the questions; the core decides which are real.
  it("asks the agent for questions and shows its wording", async () => {
    vi.mocked(commands.profileGapsStart).mockImplementation(() =>
      ok(
        outcome([
          {
            key: "eligibility",
            kind: "eligibility",
            optional: false,
            text_en: "Which countries can you work in today?",
            text_es: null,
          },
        ]),
      ),
    );
    renderStep();
    fireEvent.click(screen.getByRole("button", { name: "Ask Boba for questions" }));

    expect(await screen.findByText("Which countries can you work in today?")).toBeTruthy();
    expect(commands.profileGapsStart).toHaveBeenCalledWith(null, expect.any(String));
  });

  // AC1: the answer is the user's own fact, posted in its tagged shape.
  it("saves an eligibility answer as the user's own fact", async () => {
    renderStep();
    await screen.findByText("Where can you legally work without a new visa?");

    fireEvent.change(screen.getByLabelText("Country or region"), {
      target: { value: "es" },
    });
    fireEvent.change(screen.getByLabelText("What lets you work there?"), {
      target: { value: "citizenship" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save answer" }));

    await waitFor(() =>
      expect(commands.profileGapsAnswer).toHaveBeenCalledWith(
        "eligibility",
        { kind: "eligibility", region: "es", permit: "citizenship" },
        expect.any(String),
      ),
    );
    expect(await screen.findByText("Saved.")).toBeTruthy();
  });

  // AC1: skipping is allowed, and the core records it (the Gates drop a step).
  it("skips a question", async () => {
    renderStep();
    await screen.findByText("Where can you legally work without a new visa?");
    fireEvent.click(screen.getByRole("button", { name: "Skip" }));

    await waitFor(() => expect(commands.profileGapsSkip).toHaveBeenCalledWith("eligibility"));
    expect(
      await screen.findByText(
        "Skipped. Anything that depended on this answer is a little less certain now.",
      ),
    ).toBeTruthy();
  });

  // AC2: a fact with one variant gets the other drafted, bound to its item.
  it("drafts the missing language for a fact", async () => {
    vi.mocked(commands.profileSnapshot).mockImplementation(() => ok(snapshot([fact()])));
    vi.mocked(commands.profileGapsTranslate).mockImplementation(() =>
      ok(
        translated([
          proposal({
            id: "prop_es",
            text_en: "Notice period: 1 month",
            text_es: "Preaviso: 1 mes",
          }),
        ]),
      ),
    );
    renderStep();

    fireEvent.click(await screen.findByRole("button", { name: "Draft ES" }));
    await waitFor(() =>
      expect(commands.profileGapsTranslate).toHaveBeenCalledWith(
        "item_1",
        "es",
        expect.any(String),
      ),
    );
    expect(await screen.findByText("Preaviso: 1 mes")).toBeTruthy();
  });

  // AC3: the full proposed text, verbatim and as plain text; nothing is saved
  // until the user confirms.
  it("shows the drafted text verbatim and saves only on confirm", async () => {
    const long =
      "Preaviso: 1 mes\nQuiero un rol donde aprenda cada día y pueda aportar desde el primer sprint.";
    vi.mocked(commands.profileSnapshot).mockImplementation(() => ok(snapshot([fact()])));
    vi.mocked(commands.profileGapsTranslate).mockImplementation(() =>
      ok(translated([proposal({ id: "prop_es", text_en: null, text_es: long })])),
    );
    renderStep();

    fireEvent.click(await screen.findByRole("button", { name: "Draft ES" }));
    // The newline is part of the text (AC3: verbatim), so it is matched raw.
    const [text] = await screen.findAllByText(long, { normalizer: (value) => value });
    expect(text.tagName).toBe("P");
    expect(text.className).toContain("whitespace-pre-wrap");
    expect(text.className).not.toContain("truncate");
    expect(commands.profileGapsConfirm).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    await waitFor(() => expect(commands.profileGapsConfirm).toHaveBeenCalledWith("prop_es", null));
  });

  // AC3: the user can edit the proposal before it is saved.
  it("saves an edited draft exactly as edited", async () => {
    vi.mocked(commands.profileGapsStart).mockImplementation(() => ok(outcome([], [proposal()])));
    renderStep();
    fireEvent.click(screen.getByRole("button", { name: "Ask Boba for questions" }));

    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText("Spanish"), {
      target: { value: "Preaviso: dos meses" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save edit" }));

    await waitFor(() =>
      expect(commands.profileGapsConfirm).toHaveBeenCalledWith("prop_1", {
        text_en: "Notice period: 1 month",
        text_es: "Preaviso: dos meses",
        evidence: null,
        detail_json: "{}",
      }),
    );
  });

  // S3.3 AC2/AC4: with no working CLI the interview's AI actions are disabled
  // by the shared <Gated> state, and the manual questions stay answerable.
  it("disables the AI actions and keeps the questions answerable in Limited mode", async () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() => ok(capabilities(false)));
    vi.mocked(commands.profileSnapshot).mockImplementation(() => ok(snapshot([fact()])));
    renderStep();

    expect(await screen.findByText("AI features are off — no Agent CLI connected.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Ask Boba for questions" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Draft ES" })).toBeDisabled();
    // The answer itself is the user's own fact, not an AI action.
    expect(screen.getByRole("button", { name: "Save answer" })).toBeEnabled();
  });

  // AC4: a proposal the run made from third-party text says so.
  it("flags a proposal that came from untrusted text", async () => {
    vi.mocked(commands.profileGapsStart).mockImplementation(() =>
      ok(outcome([], [proposal({ untrusted_origin: true })])),
    );
    renderStep();
    fireEvent.click(screen.getByRole("button", { name: "Ask Boba for questions" }));

    expect(
      await screen.findByText("Boba read this from text it didn't write. Check it before saving."),
    ).toBeTruthy();
  });
});
