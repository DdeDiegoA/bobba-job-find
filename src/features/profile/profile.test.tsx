import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type FactView, type ProfileSnapshot } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import ProfileIndex from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    profileSnapshot: vi.fn(),
    profileSaveFact: vi.fn(),
    profileHistory: vi.fn(),
    profileRestore: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

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
      <ProfileIndex />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.profileSnapshot).mockReset();
  vi.mocked(commands.profileSaveFact).mockReset();
  vi.mocked(commands.profileHistory).mockReset();
  vi.mocked(commands.profileRestore).mockReset();
  vi.mocked(commands.profileSnapshot).mockImplementation(() => ok(snapshot()));
  vi.mocked(commands.profileSaveFact).mockImplementation(() => ok(snapshot()));
  vi.mocked(commands.profileHistory).mockImplementation(() => ok([]));
  vi.mocked(commands.profileRestore).mockImplementation(() => ok(snapshot()));
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
