import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OfferCardView } from "@/lib/ipc/bindings";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import { UNDO_WINDOW_MS } from "../actions";
import { rememberCard } from "./dom";
import InboxKeyboard from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    offersInbox: vi.fn(),
    inboxSave: vi.fn(),
    inboxDismiss: vi.fn(),
    inboxUndoDismiss: vi.fn(),
  },
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
    amount_minor: null,
    currency: null,
    pay_period: null,
    sources: [],
    first_seen: "2026-10-10T00:00:00Z",
    posted_at: null,
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

/** The list S6.4 draws: each card is a link to its Offer (and a couple of buttons). */
function renderTriage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const result = render(
    <QueryClientProvider client={client}>
      <main id="main">
        <InboxKeyboard />
        <article>
          <a href="#/offer/pst_01" aria-label="Open Senior Data Engineer">
            Senior Data Engineer
          </a>
          <button type="button">Save this offer</button>
        </article>
        <article>
          <a href="#/offer/pst_02" aria-label="Open Platform Engineer">
            Platform Engineer
          </a>
        </article>
        <article>
          <a href="#/offer/pst_03" aria-label="Open Junior Tester">
            Junior Tester
          </a>
        </article>
      </main>
    </QueryClientProvider>,
  );
  return result;
}

const link = (id: string) => document.querySelector(`a[href="#/offer/${id}"]`);

beforeEach(() => {
  vi.mocked(commands.offersInbox).mockReset();
  vi.mocked(commands.inboxSave).mockReset();
  vi.mocked(commands.inboxDismiss).mockReset();
  vi.mocked(commands.inboxUndoDismiss).mockReset();
  vi.mocked(commands.offersInbox).mockReturnValue(
    ok([
      card(),
      card({ posting_id: "pst_02", title: "Platform Engineer" }),
      card({ posting_id: "pst_03", title: "Junior Tester", verdict: null }),
    ]),
  );
  vi.mocked(commands.inboxSave).mockReturnValue(ok({ application_id: "app_1", created: true }));
  vi.mocked(commands.inboxDismiss).mockReturnValue(
    ok({ dismissed: ["pst_01"], failed: [], suggestion: null }),
  );
  vi.mocked(commands.inboxUndoDismiss).mockReturnValue(ok(true));
  rememberCard(null);
});

describe("J/K move the cursor (AC1)", () => {
  it("enters at the first card and walks forwards and backwards", () => {
    renderTriage();

    fireEvent.keyDown(window, { key: "j" });
    expect(document.activeElement).toBe(link("pst_01"));

    fireEvent.keyDown(window, { key: "j" });
    expect(document.activeElement).toBe(link("pst_02"));

    fireEvent.keyDown(window, { key: "k" });
    expect(document.activeElement).toBe(link("pst_01"));

    // K from the first card stays there; the ends do not wrap.
    fireEvent.keyDown(window, { key: "k" });
    expect(document.activeElement).toBe(link("pst_01"));
  });

  it("announces the Verdict as text when the cursor lands (NFR-15)", async () => {
    renderTriage();
    fireEvent.keyDown(window, { key: "j" });

    const status = screen.getByRole("status");
    await waitFor(() =>
      expect(status.textContent).toContain(
        "Senior Data Engineer. Verdict: Strong. Fit 81. Quality 71.",
      ),
    );
  });
});

describe("S saves and D dismisses the focused card (AC1)", () => {
  it("Saves the Offer under the cursor", async () => {
    renderTriage();
    fireEvent.keyDown(window, { key: "j" });
    fireEvent.keyDown(window, { key: "j" });
    fireEvent.keyDown(window, { key: "s" });

    await waitFor(() => expect(commands.inboxSave).toHaveBeenCalledWith("pst_02"));
  });

  it("still acts while the focus is on the card's own buttons", async () => {
    renderTriage();
    fireEvent.keyDown(window, { key: "j" });
    screen.getByRole("button", { name: "Save this offer" }).focus();

    fireEvent.keyDown(window, { key: "s" });
    await waitFor(() => expect(commands.inboxSave).toHaveBeenCalledWith("pst_01"));
  });

  it("Dismisses without a reason and Cmd/Ctrl+Z brings it back", async () => {
    renderTriage();
    fireEvent.keyDown(window, { key: "j" });
    fireEvent.keyDown(window, { key: "d" });

    await waitFor(() => expect(commands.inboxDismiss).toHaveBeenCalledWith(["pst_01"], null));

    fireEvent.keyDown(window, { key: "z", metaKey: true });
    await waitFor(() => expect(commands.inboxUndoDismiss).toHaveBeenCalledWith("pst_01"));
  });

  it("stops undoing once the 6-second window has passed (R1)", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      vi.setSystemTime(new Date("2026-10-17T10:00:00Z"));
      renderTriage();
      fireEvent.keyDown(window, { key: "j" });
      fireEvent.keyDown(window, { key: "d" });
      await waitFor(() => expect(commands.inboxDismiss).toHaveBeenCalledWith(["pst_01"], null));

      // Past the toast's Undo, Cmd/Ctrl+Z is an ordinary undo again.
      vi.setSystemTime(new Date(Date.now() + UNDO_WINDOW_MS + 1));
      fireEvent.keyDown(window, { key: "z", metaKey: true });
      await new Promise((resolve) => setTimeout(resolve, 0));

      expect(commands.inboxUndoDismiss).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("leaves the cursor where it was after a dismissal", async () => {
    renderTriage();
    fireEvent.keyDown(window, { key: "j" });
    fireEvent.keyDown(window, { key: "d" });

    await waitFor(() => expect(document.activeElement).toBe(link("pst_02")));
  });
});

describe("Enter opens and focus is restored (AC1)", () => {
  it("puts the cursor on the card's own link, which is what Enter follows", () => {
    renderTriage();
    fireEvent.keyDown(window, { key: "j" });

    const focused = document.activeElement as HTMLAnchorElement;
    expect(focused.tagName).toBe("A");
    expect(focused.getAttribute("href")).toBe("#/offer/pst_01");
  });

  it("comes back to the same card after leaving and returning", async () => {
    const first = renderTriage();
    fireEvent.keyDown(window, { key: "j" });
    fireEvent.keyDown(window, { key: "j" });
    expect(document.activeElement).toBe(link("pst_02"));

    // Leave the Inbox for an Offer and come back: the cursor returns.
    first.unmount();
    renderTriage();
    await waitFor(() => expect(document.activeElement).toBe(link("pst_02")));
  });
});

describe("typing is never triage (AC1)", () => {
  it("ignores the keys in a text field", async () => {
    renderTriage();
    const input = document.createElement("input");
    document.body.append(input);
    input.focus();

    fireEvent.keyDown(input, { key: "s" });
    fireEvent.keyDown(input, { key: "d" });

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(commands.inboxSave).not.toHaveBeenCalled();
    expect(commands.inboxDismiss).not.toHaveBeenCalled();
    input.remove();
  });
});
