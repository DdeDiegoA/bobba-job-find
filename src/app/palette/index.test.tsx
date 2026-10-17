import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OfferCardView } from "@/lib/ipc/bindings";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import CommandPalette from "./index";
import { openPalette, usePaletteStore } from "./store";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: { offersInbox: vi.fn() },
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

function renderPalette() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={["/"]}>
        <CommandPalette />
        <Routes>
          <Route path="/" element={<div>today screen</div>} />
          <Route path="/tracker" element={<div>tracker screen</div>} />
          <Route path="/offer/:postingId" element={<div>offer screen</div>} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.offersInbox).mockReset();
  vi.mocked(commands.offersInbox).mockReturnValue(ok([card()]));
  usePaletteStore.setState({ open: false });
});

describe("opening and closing (AC2)", () => {
  it("opens on Cmd/Ctrl+K, lists every page and closes on Esc", () => {
    renderPalette();
    expect(screen.queryByRole("dialog")).toBeNull();

    fireEvent.keyDown(window, { key: "k", metaKey: true });

    const dialog = screen.getByRole("dialog", { name: "Command palette" });
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    // The field takes the focus, so the user can type immediately.
    expect(document.activeElement).toBe(screen.getByRole("combobox"));
    for (const page of ["Today", "Inbox", "Studio", "Tracker", "Settings"]) {
      expect(screen.getByRole("option", { name: new RegExp(page) })).toBeTruthy();
    }

    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("opens when the topbar trigger calls openPalette()", async () => {
    renderPalette();
    openPalette();
    expect(await screen.findByRole("dialog")).toBeTruthy();
  });
});

describe("navigating (AC2)", () => {
  it("filters the pages and opens the chosen one on Enter", async () => {
    renderPalette();
    fireEvent.keyDown(window, { key: "k", metaKey: true });
    const dialog = screen.getByRole("dialog");

    fireEvent.change(screen.getByRole("combobox"), { target: { value: "trac" } });
    const options = screen.getAllByRole("option");
    expect(options).toHaveLength(1);
    expect(options[0].textContent).toContain("Tracker");

    fireEvent.keyDown(dialog, { key: "Enter" });
    expect(await screen.findByText("tracker screen")).toBeTruthy();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("searches Offers by title or company and opens the chosen one", async () => {
    renderPalette();
    fireEvent.keyDown(window, { key: "k", metaKey: true });
    const dialog = screen.getByRole("dialog");

    fireEvent.change(screen.getByRole("combobox"), { target: { value: "boba" } });
    const offer = await screen.findByRole("option", { name: /Senior Data Engineer/ });
    expect(offer.textContent).toContain("Boba Labs");

    fireEvent.keyDown(dialog, { key: "ArrowDown" });
    // Down past the pages that do not match: "Boba" matches no page, so the
    // Offer is already the active row.
    fireEvent.keyDown(dialog, { key: "Enter" });
    expect(await screen.findByText("offer screen")).toBeTruthy();
  });

  it("says so when nothing matches", async () => {
    renderPalette();
    fireEvent.keyDown(window, { key: "k", metaKey: true });
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "zzzz" } });
    await waitFor(() => expect(screen.getByText("No matches")).toBeTruthy());
  });

  it("gives the focus back when it closes", () => {
    renderPalette();
    const trigger = document.createElement("button");
    trigger.textContent = "trigger";
    document.body.append(trigger);
    trigger.focus();

    fireEvent.keyDown(window, { key: "k", metaKey: true });
    expect(document.activeElement).toBe(screen.getByRole("combobox"));

    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(document.activeElement).toBe(trigger);
    trigger.remove();
  });
});
