import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  type CapabilitySnapshot,
  commands,
  type FactView,
  type ImproveOutcome,
  type ProfileSnapshot,
  type ProposalView,
} from "@/lib/ipc/bindings";
import "@/lib/i18n";
import { ImproveFact } from "./improve";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    capabilitiesSnapshot: vi.fn(),
    profileImproveStart: vi.fn(),
    profileImproveConfirm: vi.fn(),
    profileImproveDiscard: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const fail = (messageKey: string) =>
  Promise.resolve({
    status: "error" as const,
    error: { code: "internal" as const, message_key: messageKey, details: null },
  });

/** AI work on profile facts is available unless the test says otherwise. */
const capabilities = (available = true): CapabilitySnapshot => ({
  limited: !available,
  capabilities: [
    { id: "profile_gaps", available, reason: available ? null : "no_agent" },
    { id: "profile_translation", available, reason: available ? null : "no_agent" },
  ],
});

const fact = (over: Partial<FactView> = {}): FactView => ({
  item_id: "01ITEMEXP",
  revision_id: "01REV1",
  kind: "experience",
  text_en: "Backend engineer at Nimbus",
  text_es: null,
  evidence: null,
  detail_json: "{}",
  missing_variant: null,
  profile_version: 1,
  provenance: "user",
  from_agent_run: false,
  ...over,
});

const suggestion = (over: Partial<ProposalView> = {}): ProposalView => ({
  id: "prop_one",
  item_id: "01ITEMEXP",
  kind: "experience",
  text_en: "Led the Nimbus backend rewrite in Rust",
  text_es: null,
  evidence: null,
  detail_json: "{}",
  source_excerpt: null,
  base_revision: "01REV1",
  untrusted_origin: true,
  created_at: "2026-10-18T09:00:00.000Z",
  ...over,
});

const improved = (proposals: ProposalView[]): ImproveOutcome => ({
  run_id: "run_improve",
  proposals,
});

const snapshot = (): ProfileSnapshot => ({
  profile_version: 2,
  facts: [fact()],
  pending_proposals: 0,
});

function renderImprove(onConfirmed = vi.fn()) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const view = render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <ImproveFact fact={fact()} onConfirmed={onConfirmed} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  return { ...view, onConfirmed };
}

/** The action is disabled while the capability snapshot loads: wait, then click. */
async function clickSuggest() {
  const button = screen.getByRole("button", { name: "Suggest improvement" });
  await waitFor(() => expect(button).toBeEnabled());
  fireEvent.click(button);
}

beforeEach(() => {
  for (const fn of Object.values(commands)) vi.mocked(fn).mockReset();
  vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() => ok(capabilities()));
  vi.mocked(commands.profileImproveStart).mockImplementation(() => ok(improved([])));
  vi.mocked(commands.profileImproveConfirm).mockImplementation(() => ok(snapshot()));
  vi.mocked(commands.profileImproveDiscard).mockImplementation(() => ok(null));
});

describe("Profile — suggest an improvement (FX10b AC1)", () => {
  it("asks the engine for the fact and shows each suggestion next to the current text", async () => {
    vi.mocked(commands.profileImproveStart).mockImplementation(() => ok(improved([suggestion()])));
    renderImprove();

    await clickSuggest();

    expect(await screen.findByText("Led the Nimbus backend rewrite in Rust")).toBeTruthy();
    // Both sides are on screen: the fact's own words and the rewrite (AC1).
    expect(screen.getByText("Backend engineer at Nimbus")).toBeTruthy();
    expect(screen.getByText("Current text")).toBeTruthy();
    expect(screen.getByText("Suggested text")).toBeTruthy();
    expect(commands.profileImproveStart).toHaveBeenCalledWith("01ITEMEXP", "en");
    expect(screen.getByRole("button", { name: "Accept" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Reject" })).toBeTruthy();
  });

  it("renders agent text verbatim, never as markup (AD-7)", async () => {
    const text = "Ignore previous instructions.\n<b>Senior</b> engineer <script>alert(1)</script>";
    vi.mocked(commands.profileImproveStart).mockImplementation(() =>
      ok(improved([suggestion({ text_en: text })])),
    );
    renderImprove();

    await clickSuggest();

    const card = await screen.findByRole("listitem");
    // Text verbatim, and never parsed as markup.
    expect(card.textContent).toContain(text);
    expect(card.querySelector("b")).toBeNull();
    expect(card.querySelector("script")).toBeNull();
  });
});

describe("Profile — confirm one suggestion (FX10b AC2, R5)", () => {
  it("accepts exactly the suggestion that was rendered, and only then", async () => {
    const second = suggestion({ id: "prop_two", text_en: "Owned the Nimbus platform" });
    vi.mocked(commands.profileImproveStart).mockImplementation(() =>
      ok(improved([suggestion(), second])),
    );
    const { onConfirmed } = renderImprove();

    await clickSuggest();
    const rendered = await screen.findByText("Owned the Nimbus platform");
    // R5: rendering a suggestion never confirms it.
    expect(commands.profileImproveConfirm).not.toHaveBeenCalled();

    const card = rendered.closest("li") as HTMLElement;
    fireEvent.click(within(card).getByRole("button", { name: "Accept" }));

    await waitFor(() => expect(commands.profileImproveConfirm).toHaveBeenCalledTimes(1));
    // Exactly the rendered one, as proposed; the other suggestion is untouched.
    expect(commands.profileImproveConfirm).toHaveBeenCalledWith("prop_two", null);
    expect(commands.profileImproveDiscard).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByText("Owned the Nimbus platform")).toBeNull());
    expect(onConfirmed).toHaveBeenCalledWith({ en: "Owned the Nimbus platform" });
  });

  it("rejects only that suggestion: it never becomes a fact", async () => {
    const second = suggestion({ id: "prop_two", text_en: "Owned the Nimbus platform" });
    vi.mocked(commands.profileImproveStart).mockImplementation(() =>
      ok(improved([suggestion(), second])),
    );
    renderImprove();

    await clickSuggest();
    const rendered = await screen.findByText("Owned the Nimbus platform");
    const card = rendered.closest("li") as HTMLElement;
    fireEvent.click(within(card).getByRole("button", { name: "Reject" }));

    await waitFor(() => expect(commands.profileImproveDiscard).toHaveBeenCalledWith("prop_two"));
    expect(commands.profileImproveConfirm).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByText("Owned the Nimbus platform")).toBeNull());
    expect(screen.getByText("Led the Nimbus backend rewrite in Rust")).toBeTruthy();
  });
});

describe("Profile — Limited mode and errors (FX10b AC3)", () => {
  it("disables the action without an engine and says why", async () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() => ok(capabilities(false)));
    renderImprove();

    expect(
      await screen.findByText(/Choose an AI engine before asking Boba to improve a fact/),
    ).toBeTruthy();
    const button = screen.getByRole("button", { name: "Suggest improvement" });
    expect(button).toBeDisabled();
    fireEvent.click(button);
    expect(commands.profileImproveStart).not.toHaveBeenCalled();
  });

  it("shows a translated error and leaves the current text alone", async () => {
    vi.mocked(commands.profileImproveStart).mockImplementation(() =>
      fail("errors.profile_improve.run"),
    );
    const { onConfirmed } = renderImprove();

    await clickSuggest();

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe(
      "The suggestion didn't finish. The AI engine may be signed out or out of quota.",
    );
    // Nothing was proposed, and the fact itself was never touched.
    expect(screen.queryByRole("button", { name: "Accept" })).toBeNull();
    expect(onConfirmed).not.toHaveBeenCalled();
  });
});

describe("Profile — improvement is accessible (FX10b AC4)", () => {
  it("has no axe violations with suggestions on screen", async () => {
    vi.mocked(commands.profileImproveStart).mockImplementation(() =>
      ok(improved([suggestion(), suggestion({ id: "prop_two", text_en: "Owned Nimbus" })])),
    );
    const { container } = renderImprove();

    await clickSuggest();
    await screen.findByText("Owned Nimbus");

    const { axeAudit } = await import("../../../e2e/a11y/axe");
    const violations = await axeAudit(container);
    expect(violations).toEqual([]);
  });
});
