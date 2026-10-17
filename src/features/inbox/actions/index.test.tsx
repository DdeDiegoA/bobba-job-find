import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { toast } from "sonner";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OfferCardView, SuggestionView } from "@/lib/ipc/bindings";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import { inboxKey } from "../list/offers";
import { InboxBulkBar, OfferActions, SuggestionBanner, UNDO_WINDOW_MS } from ".";
import { useBulkStore } from "./bulk-store";
import { useSuggestionStore } from "./suggestion-store";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    inboxSave: vi.fn(),
    inboxDismiss: vi.fn(),
    inboxUndoDismiss: vi.fn(),
    inboxOverrideVeto: vi.fn(),
    inboxDecideSuggestion: vi.fn(),
  },
}));

vi.mock("sonner", () => ({
  toast: Object.assign(
    vi.fn(() => "toast-1"),
    { error: vi.fn() },
  ),
  Toaster: () => null,
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

function card(overrides: Partial<OfferCardView> = {}): OfferCardView {
  return {
    posting_id: "pst_01",
    title: "Senior Data Engineer",
    company: "Boba Labs",
    locations: ["Remote, Spain"],
    work_mode: "remote",
    lang: "en",
    amount_minor: 5_500_000,
    currency: "EUR",
    pay_period: "year",
    sources: ["greenhouse"],
    first_seen: "2026-10-10T00:00:00Z",
    posted_at: "2026-10-10T00:00:00Z",
    deadline: null,
    fit: 81,
    quality: 71,
    rank: 78,
    verdict: "strong",
    confidence: "medium",
    lead: false,
    badges: [],
    exclusion: null,
    closing_soon: false,
    dismissed: false,
    overridden: false,
    ...overrides,
  };
}

function renderWith(node: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const result = render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
  return { ...result, client };
}

beforeEach(() => {
  vi.clearAllMocks();
  useBulkStore.setState({ active: false, selected: [] });
  useSuggestionStore.setState({ current: null });
  vi.mocked(commands.inboxSave).mockReturnValue(ok({ application_id: "app_01", created: true }));
  vi.mocked(commands.inboxDismiss).mockReturnValue(
    ok({ dismissed: ["pst_01"], failed: [], suggestion: null }),
  );
  vi.mocked(commands.inboxUndoDismiss).mockReturnValue(ok(true));
  vi.mocked(commands.inboxOverrideVeto).mockReturnValue(
    ok({ posting_id: "pst_01", gate: "eligibility" }),
  );
  vi.mocked(commands.inboxDecideSuggestion).mockReturnValue(ok(null));
});

describe("Save (AC1)", () => {
  it("asks the core for an Application and confirms it", async () => {
    renderWith(<OfferActions card={card()} />);

    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await vi.waitFor(() => expect(commands.inboxSave).toHaveBeenCalledWith("pst_01"));
    await vi.waitFor(() => expect(toast).toHaveBeenCalledWith("Saved to your Tracker"));
  });
});

describe("Dismiss (AC1)", () => {
  it("sends the reason the user picked", async () => {
    renderWith(<OfferActions card={card()} />);

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "The company" }));

    await vi.waitFor(() =>
      expect(commands.inboxDismiss).toHaveBeenCalledWith(["pst_01"], "company"),
    );
  });

  it("dismisses without a reason when the user says no more", async () => {
    renderWith(<OfferActions card={card()} />);

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Just dismiss" }));

    await vi.waitFor(() => expect(commands.inboxDismiss).toHaveBeenCalledWith(["pst_01"], null));
  });

  it("offers Undo for 6 seconds, and Undo calls the core", async () => {
    renderWith(<OfferActions card={card()} />);

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Not interested" }));

    await vi.waitFor(() => expect(toast).toHaveBeenCalled());
    const [, options] = vi.mocked(toast).mock.calls[0] as unknown as [
      string,
      { duration: number; action: { label: string; onClick: () => void } },
    ];
    expect(options.duration).toBe(UNDO_WINDOW_MS);
    expect(options.duration).toBe(6000);
    expect(options.action.label).toBe("Undo");

    options.action.onClick();
    await vi.waitFor(() => expect(commands.inboxUndoDismiss).toHaveBeenCalledWith("pst_01"));
  });

  // S6.6 R-1: a batch that failed halfway still dismissed some Offers.
  it("re-reads the Inbox when a dismissal fails halfway", async () => {
    vi.mocked(commands.inboxDismiss).mockReturnValue(
      Promise.resolve({
        status: "error" as const,
        error: {
          code: "internal",
          message_key: "errors.inbox_actions.store",
          details: null,
        },
      }) as never,
    );
    const { client } = renderWith(<OfferActions card={card()} />);
    const invalidate = vi.spyOn(client, "invalidateQueries");

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "The company" }));

    await vi.waitFor(() => expect(invalidate).toHaveBeenCalledWith({ queryKey: inboxKey }));
  });

  it("keeps the card when the core refuses the dismissal", async () => {
    vi.mocked(commands.inboxDismiss).mockReturnValue(
      Promise.resolve({
        status: "error" as const,
        error: { code: "internal", message_key: "errors.inbox_actions.store", details: null },
      }) as never,
    );
    renderWith(<OfferActions card={card()} />);

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "The pay" }));

    await vi.waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith(
        "Boba could not read or write your Inbox decisions.",
      ),
    );
  });
});

describe("Excluded override (AC2)", () => {
  const excluded = card({
    exclusion: {
      gate: "eligibility",
      reason: "requirement_not_met",
      quote: "Must be authorized to work in the US.",
    },
  });

  it("offers Override on an Excluded Offer and records it", async () => {
    renderWith(<OfferActions card={excluded} />);

    fireEvent.click(screen.getByRole("button", { name: "Override veto" }));

    await vi.waitFor(() => expect(commands.inboxOverrideVeto).toHaveBeenCalledWith("pst_01"));
    await vi.waitFor(() =>
      expect(toast).toHaveBeenCalledWith("Override recorded — the Offer is shown again."),
    );
  });

  it("never offers Override on an Offer no Gate vetoed", () => {
    renderWith(<OfferActions card={card()} />);
    expect(screen.queryByRole("button", { name: "Override veto" })).toBeNull();
  });

  it("does not offer Override twice", () => {
    renderWith(<OfferActions card={{ ...excluded, overridden: true }} />);
    expect(screen.queryByRole("button", { name: "Override veto" })).toBeNull();
  });
});

describe("Bulk dismiss (AC1)", () => {
  const cards = [
    card({ posting_id: "pst_01", title: "Data Engineer" }),
    card({ posting_id: "pst_02", title: "Analytics Engineer" }),
  ];

  it("picks cards and dismisses them in one call", async () => {
    vi.mocked(commands.inboxDismiss).mockReturnValue(
      ok({ dismissed: ["pst_01", "pst_02"], failed: [], suggestion: null }),
    );
    renderWith(<InboxBulkBar cards={cards} />);

    fireEvent.click(screen.getByRole("button", { name: "Select" }));
    fireEvent.click(screen.getByRole("button", { name: "Select all shown" }));
    fireEvent.click(screen.getByRole("button", { name: "Dismiss 2" }));

    await vi.waitFor(() =>
      expect(commands.inboxDismiss).toHaveBeenCalledWith(["pst_01", "pst_02"], null),
    );
    await vi.waitFor(() => expect(toast).toHaveBeenCalledWith("Dismissed 2"));
  });

  it("shows a checkbox on every card while the mode is on", () => {
    renderWith(
      <>
        <InboxBulkBar cards={cards} />
        {cards.map((item) => (
          <OfferActions key={item.posting_id} card={item} />
        ))}
      </>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Select" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Data Engineer" }));

    expect(useBulkStore.getState().selected).toEqual(["pst_01"]);
  });
});

describe("Suggestion (AC3)", () => {
  const suggestion: SuggestionView = {
    id: "exclude_company:boba labs",
    kind: "exclude_company",
    company: "Boba Labs",
    count: 3,
  };

  it("shows the sentence the core suggested and accepts it", async () => {
    useSuggestionStore.setState({ current: suggestion });
    renderWith(<SuggestionBanner />);

    expect(
      screen.getByText("You dismissed 3 roles at Boba Labs. Exclude this company?"),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Yes, do it" }));

    await vi.waitFor(() =>
      expect(commands.inboxDecideSuggestion).toHaveBeenCalledWith(
        "exclude_company:boba labs",
        true,
        "Boba Labs",
      ),
    );
    await vi.waitFor(() => expect(useSuggestionStore.getState().current).toBeNull());
  });

  it("can be ignored without changing Preferences", async () => {
    useSuggestionStore.setState({ current: suggestion });
    renderWith(<SuggestionBanner />);

    fireEvent.click(screen.getByRole("button", { name: "No, ignore" }));

    await vi.waitFor(() =>
      expect(commands.inboxDecideSuggestion).toHaveBeenCalledWith(
        "exclude_company:boba labs",
        false,
        "Boba Labs",
      ),
    );
  });

  it("renders nothing while no suggestion is on offer", () => {
    renderWith(<SuggestionBanner />);
    expect(screen.queryByRole("complementary")).toBeNull();
  });
});
