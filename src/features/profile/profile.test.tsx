import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  type CapabilitySnapshot,
  commands,
  type FactView,
  type ProfileSnapshot,
  type ProposalView,
  type TranslateOutcome,
} from "@/lib/ipc/bindings";
import "@/lib/i18n";
import ProfileIndex from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    capabilitiesSnapshot: vi.fn(),
    profileSnapshot: vi.fn(),
    profileSaveFact: vi.fn(),
    profileHistory: vi.fn(),
    profileRestore: vi.fn(),
    profileGapsTranslate: vi.fn(),
    profileGapsConfirm: vi.fn(),
    profileGapsDiscard: vi.fn(),
    profileImportProposals: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

/** The capabilities the core reports (FX8 AC4): AI on unless the test says no. */
const capabilities = (available = true): CapabilitySnapshot => ({
  limited: !available,
  capabilities: [
    { id: "profile_gaps", available, reason: available ? null : "no_agent" },
    { id: "profile_translation", available, reason: available ? null : "no_agent" },
  ],
});

const proposal = (over: Partial<ProposalView> = {}): ProposalView => ({
  id: "prop_es",
  item_id: "01ITEMEXP",
  kind: "experience",
  text_en: "Senior Backend Engineer",
  text_es: "Ingeniero Backend Senior",
  evidence: null,
  detail_json: "{}",
  source_excerpt: null,
  base_revision: null,
  untrusted_origin: false,
  created_at: "2026-10-18T09:00:00.000Z",
  ...over,
});

const translated = (proposals: ProposalView[]): TranslateOutcome => ({
  run_id: "run_1",
  proposals,
});

const fact = (over: Partial<FactView>): FactView => ({
  item_id: "01ITEMEXP",
  revision_id: "01REV2",
  kind: "experience",
  text_en: "Senior Backend Engineer",
  text_es: null,
  evidence: null,
  detail_json: "{}",
  missing_variant: "es",
  profile_version: 2,
  provenance: "user",
  from_agent_run: false,
  ...over,
});

const snapshot = (over: Partial<ProfileSnapshot> = {}): ProfileSnapshot => ({
  profile_version: 2,
  facts: [fact({})],
  pending_proposals: 0,
  ...over,
});

function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={["/profile"]}>
        <ProfileIndex />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  for (const fn of Object.values(commands)) vi.mocked(fn).mockReset();
  vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() => ok(capabilities()));
  vi.mocked(commands.profileSnapshot).mockImplementation(() => ok(snapshot()));
  vi.mocked(commands.profileSaveFact).mockImplementation(() => ok(snapshot()));
  vi.mocked(commands.profileHistory).mockImplementation(() => ok([]));
  vi.mocked(commands.profileRestore).mockImplementation(() => ok(snapshot()));
  vi.mocked(commands.profileGapsTranslate).mockImplementation(() => ok(translated([])));
  vi.mocked(commands.profileGapsConfirm).mockImplementation(() => ok(snapshot()));
  vi.mocked(commands.profileGapsDiscard).mockImplementation(() => ok(null));
  vi.mocked(commands.profileImportProposals).mockImplementation(() => ok([]));
});

describe("Profile — facts and language variants (AC1/AC2)", () => {
  it("shows the facts of the selected section", async () => {
    renderScreen();
    expect(await screen.findByText("Senior Backend Engineer")).toBeTruthy();
    expect(screen.getByText("v2")).toBeTruthy();
  });

  it("flags the missing ES variant instead of inventing one", async () => {
    renderScreen();
    expect(await screen.findByText(/ES variant missing/)).toBeTruthy();
  });

  it("returns to an empty state when the section has no facts", async () => {
    vi.mocked(commands.profileSnapshot).mockImplementation(() => ok(snapshot({ facts: [] })));
    renderScreen();
    expect(await screen.findByText(/Add your first Experience/)).toBeTruthy();
  });

  it("counts pending proposals from the snapshot (AC4)", async () => {
    vi.mocked(commands.profileSnapshot).mockImplementation(() =>
      ok(snapshot({ pending_proposals: 2 })),
    );
    renderScreen();
    expect(await screen.findByText(/2 suggested facts/)).toBeTruthy();
  });
});

describe("Profile — saving creates a version (AC3)", () => {
  it("sends an edit of the existing item, so the core appends a revision", async () => {
    renderScreen();
    await screen.findByText("Senior Backend Engineer");
    fireEvent.click(await screen.findByRole("button", { name: "Edit Experience" }));
    const english = screen.getByLabelText("English") as HTMLTextAreaElement;
    fireEvent.change(english, { target: { value: "Senior Backend Engineer II" } });

    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(commands.profileSaveFact).toHaveBeenCalledTimes(1));
    expect(vi.mocked(commands.profileSaveFact).mock.calls[0][0]).toMatchObject({
      item_id: "01ITEMEXP",
      kind: "experience",
      text_en: "Senior Backend Engineer II",
      text_es: null,
    });
  });

  it("creates a new item when adding to a section", async () => {
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "+ Add Experience" }));
    fireEvent.change(screen.getByLabelText("Spanish"), {
      target: { value: "Ingeniero Backend Senior" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(commands.profileSaveFact).toHaveBeenCalledTimes(1));
    expect(vi.mocked(commands.profileSaveFact).mock.calls[0][0]).toMatchObject({
      item_id: null,
      text_en: null,
      text_es: "Ingeniero Backend Senior",
    });
  });

  it("refuses a fact with no text at all (AC2)", async () => {
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "+ Add Experience" }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("Add text in English or Spanish.")).toBeTruthy();
    expect(commands.profileSaveFact).not.toHaveBeenCalled();
  });
});

describe("Profile — draft the other language with AI (FX8 AC1)", () => {
  it("flags the missing language and offers a draft when the fact is opened", async () => {
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Edit Experience" }));

    const form = await screen.findByRole("form", { name: "Edit Experience" });
    expect(within(form).getByText("⚠ ES variant missing here")).toBeTruthy();
    // The capability snapshot decides; the button stays disabled while it loads.
    await waitFor(() =>
      expect(within(form).getByRole("button", { name: "Draft in Spanish with AI" })).toBeEnabled(),
    );
  });

  it("drafts through the agent runtime and shows the proposal verbatim, unsaved", async () => {
    const long = "Ingeniero Backend Senior\nLideré la migración a Rust.";
    vi.mocked(commands.profileGapsTranslate).mockImplementation(() =>
      ok(translated([proposal({ text_es: long })])),
    );
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Edit Experience" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Draft in Spanish with AI" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Draft in Spanish with AI" }));

    await waitFor(() =>
      expect(commands.profileGapsTranslate).toHaveBeenCalledWith(
        "01ITEMEXP",
        "es",
        expect.any(String),
      ),
    );
    const [text] = await screen.findAllByText(long, { normalizer: (value) => value });
    expect(text.tagName).toBe("P");
    expect(text.className).toContain("whitespace-pre-wrap");
    // Nothing is written before the user confirms.
    expect(commands.profileGapsConfirm).not.toHaveBeenCalled();
    expect(commands.profileSaveFact).not.toHaveBeenCalled();
  });

  it("saves the confirmed draft as a new version and nothing else", async () => {
    vi.mocked(commands.profileGapsTranslate).mockImplementation(() => ok(translated([proposal()])));
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Edit Experience" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Draft in Spanish with AI" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Draft in Spanish with AI" }));
    fireEvent.click(await screen.findByRole("button", { name: "Confirm" }));

    await waitFor(() => expect(commands.profileGapsConfirm).toHaveBeenCalledWith("prop_es", null));
    expect(commands.profileSaveFact).not.toHaveBeenCalled();
    // The form now holds the confirmed text, so a later Save cannot wipe it.
    expect(screen.getByLabelText("Spanish")).toHaveValue("Ingeniero Backend Senior");
  });

  it("shows a fact's pending drafts again when the form is reopened", async () => {
    // The core keeps the draft; reopening must list it, not hide it (F1).
    const stored: ProposalView[] = [];
    vi.mocked(commands.profileImportProposals).mockImplementation(() => ok([...stored]));
    vi.mocked(commands.profileGapsTranslate).mockImplementation(() => ok(translated([proposal()])));
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Edit Experience" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Draft in Spanish with AI" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Draft in Spanish with AI" }));
    stored.push(proposal());
    await screen.findByText("Ingeniero Backend Senior");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    fireEvent.click(screen.getByRole("button", { name: "Edit Experience" }));
    expect(await screen.findByText("Ingeniero Backend Senior")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Confirm" })).toBeTruthy();
  });

  it("does not offer a confirmed draft again when the form is reopened", async () => {
    const stored: ProposalView[] = [proposal()];
    vi.mocked(commands.profileImportProposals).mockImplementation(() => ok([...stored]));
    vi.mocked(commands.profileGapsConfirm).mockImplementation(() => {
      stored.length = 0;
      return ok(snapshot());
    });
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Edit Experience" }));
    fireEvent.click(await screen.findByRole("button", { name: "Confirm" }));
    await waitFor(() => expect(commands.profileGapsConfirm).toHaveBeenCalled());
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    fireEvent.click(screen.getByRole("button", { name: "Edit Experience" }));
    const form = await screen.findByRole("form", { name: "Edit Experience" });
    await waitFor(() => expect(within(form).queryByText("Ingeniero Backend Senior")).toBeNull());
  });

  it("keeps an earlier draft when a second one is requested", async () => {
    vi.mocked(commands.profileGapsTranslate)
      .mockImplementationOnce(() =>
        ok(translated([proposal({ id: "prop_1", text_es: "Primer borrador" })])),
      )
      .mockImplementationOnce(() =>
        ok(translated([proposal({ id: "prop_2", text_es: "Segundo borrador" })])),
      );
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Edit Experience" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Draft in Spanish with AI" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Draft in Spanish with AI" }));
    await screen.findByText("Primer borrador");
    fireEvent.click(screen.getByRole("button", { name: "Draft in Spanish with AI" }));
    await screen.findByText("Segundo borrador");

    // Neither draft is orphaned: both stay actionable.
    expect(screen.getByText("Primer borrador")).toBeTruthy();
    expect(screen.getAllByRole("button", { name: "Confirm" })).toHaveLength(2);
  });

  it("stops offering the draft once the language is confirmed", async () => {
    vi.mocked(commands.profileGapsTranslate).mockImplementation(() => ok(translated([proposal()])));
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Edit Experience" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Draft in Spanish with AI" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Draft in Spanish with AI" }));
    fireEvent.click(await screen.findByRole("button", { name: "Confirm" }));

    await waitFor(() => expect(commands.profileGapsConfirm).toHaveBeenCalled());
    const form = screen.getByRole("form", { name: "Edit Experience" });
    expect(within(form).queryByRole("button", { name: "Draft in Spanish with AI" })).toBeNull();
    expect(within(form).queryByText("⚠ ES variant missing here")).toBeNull();
  });

  it("discards a draft without touching the profile", async () => {
    vi.mocked(commands.profileGapsTranslate).mockImplementation(() => ok(translated([proposal()])));
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Edit Experience" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Draft in Spanish with AI" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Draft in Spanish with AI" }));
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }));

    await waitFor(() => expect(commands.profileGapsDiscard).toHaveBeenCalledWith("prop_es"));
    expect(commands.profileGapsConfirm).not.toHaveBeenCalled();
    expect(commands.profileSaveFact).not.toHaveBeenCalled();
  });

  it("shows the translated error and leaves the profile unchanged when the run fails", async () => {
    vi.mocked(commands.profileGapsTranslate).mockImplementation(() =>
      Promise.reject({
        code: "agent_unsupported",
        message_key: "errors.agent.unsupported",
        details: null,
      }),
    );
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Edit Experience" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Draft in Spanish with AI" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Draft in Spanish with AI" }));

    // Announced as an alert, not just printed (FX8 review F2).
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/needs an AI tool with a verified tool lockdown/);
    expect(commands.profileGapsConfirm).not.toHaveBeenCalled();
    expect(commands.profileSaveFact).not.toHaveBeenCalled();
  });

  it("announces that a draft is running", async () => {
    let settle!: (value: { status: "ok"; data: TranslateOutcome }) => void;
    vi.mocked(commands.profileGapsTranslate).mockImplementation(
      () =>
        new Promise((resolve) => {
          settle = resolve;
        }),
    );
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Edit Experience" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Draft in Spanish with AI" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Draft in Spanish with AI" }));

    expect(await screen.findByRole("status")).toHaveTextContent("Drafting…");
    settle({ status: "ok", data: translated([]) });
  });

  it("offers no AI draft for a fact with both variants", async () => {
    vi.mocked(commands.profileSnapshot).mockImplementation(() =>
      ok(
        snapshot({ facts: [fact({ text_es: "Ingeniero Backend Senior", missing_variant: null })] }),
      ),
    );
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Edit Experience" }));

    const form = await screen.findByRole("form", { name: "Edit Experience" });
    expect(within(form).queryByRole("button", { name: /Draft in .* with AI/ })).toBeNull();
  });
});

describe("Profile — AI draft in Limited mode (FX8 AC4)", () => {
  it("disables the draft, explains why, and links to Settings > AI engine", async () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() => ok(capabilities(false)));
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Edit Experience" }));

    const form = await screen.findByRole("form", { name: "Edit Experience" });
    expect(
      await within(form).findByText("AI features are off — no Agent CLI connected."),
    ).toBeTruthy();
    expect(within(form).getByRole("button", { name: "Draft in Spanish with AI" })).toBeDisabled();
    expect(within(form).getByRole("link", { name: "Open Settings > AI engine" })).toBeTruthy();
  });

  it("explains the disabled draft when the capability snapshot cannot be read", async () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() =>
      Promise.reject(new Error("snapshot failed")),
    );
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "Edit Experience" }));

    const form = await screen.findByRole("form", { name: "Edit Experience" });
    expect(
      await within(form).findByText("AI features are off — no Agent CLI connected."),
    ).toBeTruthy();
    expect(within(form).getByRole("button", { name: "Draft in Spanish with AI" })).toBeDisabled();
  });

  it("navigates from Limited mode to Settings > AI engine", async () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() => ok(capabilities(false)));
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={["/profile"]}>
          <Routes>
            <Route path="/profile" element={<ProfileIndex />} />
            <Route path="/settings/ai-engine" element={<p>Engine settings</p>} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Edit Experience" }));

    const form = await screen.findByRole("form", { name: "Edit Experience" });
    fireEvent.click(within(form).getByRole("link", { name: "Open Settings > AI engine" }));
    expect(await screen.findByText("Engine settings")).toBeTruthy();
  });
});

describe("Profile — version history (AC3)", () => {
  it("lists every version and restores an earlier one", async () => {
    vi.mocked(commands.profileHistory).mockImplementation(() =>
      ok([
        fact({ revision_id: "01REV2", text_en: "v2", profile_version: 2 }),
        fact({ revision_id: "01REV1", text_en: "v1", profile_version: 1 }),
      ]),
    );
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "History" }));

    await screen.findByRole("region", { name: "History" });
    await screen.findAllByText("v1");
    // Only the older revision can be restored; the current one has no button.
    fireEvent.click(screen.getByRole("button", { name: "Restore" }));

    await waitFor(() =>
      expect(commands.profileRestore).toHaveBeenCalledWith("01ITEMEXP", "01REV1"),
    );
  });

  it("says so when there is no earlier version", async () => {
    vi.mocked(commands.profileHistory).mockImplementation(() =>
      ok([fact({ revision_id: "01REV2" })]),
    );
    renderScreen();
    fireEvent.click(await screen.findByRole("button", { name: "History" }));
    expect(await screen.findByText("No earlier versions yet.")).toBeTruthy();
  });
});
