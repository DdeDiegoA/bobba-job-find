import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import "@/lib/i18n";
import { ThemeProvider } from "@/app/theme";
import { Shell } from "@/app/shell";
import { usePaletteStore } from "@/app/palette/store";
import KillSwitch from "@/app/header/kill-switch";
import { type BoardView, commands, events } from "@/lib/ipc/bindings";
import { Board } from "@/features/tracker/board/board";
import { OnboardingWizard } from "@/features/onboarding/wizard";

/**
 * Keyboard-only operation of the AC1 screens (S10.6 AC1, NFR-15).
 *
 * jsdom does not run the browser's implicit activation (Enter/Space on a native
 * button) nor move focus on Tab, so what is asserted here is the part that
 * belongs to the app: native semantics, focusability, the app's own key
 * handlers, and the focus management the app implements itself. The physical
 * key presses are in the keyboard-only script in `docs/qa/accessibility.md`.
 */

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    trackerBoard: vi.fn(),
    trackerApplicationDetail: vi.fn(),
    trackerTransitionStatus: vi.fn(),
    killSwitchStatus: vi.fn(),
    killSwitchSet: vi.fn(),
  },
  events: {
    killSwitchChanged: { listen: vi.fn(() => Promise.resolve(() => undefined)) },
  },
}));

vi.mock("sonner", () => ({ toast: vi.fn(), Toaster: () => null }));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

function board(): BoardView {
  const empty = (stage: string): BoardView["columns"][number] => ({
    stage,
    status: stage,
    terminal: false,
    count: 0,
    cards: [],
  });
  return {
    columns: [
      {
        ...empty("saved"),
        count: 1,
        cards: [
          {
            id: "app_1",
            status: "saved",
            stage: "saved",
            title: "Backend Engineer",
            company: "Tapi Tea",
            next_action: null,
          },
        ],
      },
      empty("preparing"),
      empty("applied"),
    ],
  };
}

function providers(children: React.ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return (
    <QueryClientProvider client={client}>
      <ThemeProvider>
        <MemoryRouter>{children}</MemoryRouter>
      </ThemeProvider>
    </QueryClientProvider>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  usePaletteStore.setState({ open: false });
  localStorage.clear();
  vi.mocked(commands.trackerBoard).mockReturnValue(ok(board()));
  vi.mocked(commands.trackerApplicationDetail).mockReturnValue(ok(null) as never);
  vi.mocked(commands.trackerTransitionStatus).mockReturnValue(ok(undefined) as never);
  vi.mocked(commands.killSwitchStatus).mockReturnValue(
    ok({ engaged: false, since: null }) as never,
  );
  vi.mocked(commands.killSwitchSet).mockReturnValue(
    ok({ view: { engaged: true, since: "2026-10-17T00:00:00Z" }, parked: [] }) as never,
  );
});

describe("Shell — get to the content without a mouse", () => {
  it("puts a skip link first that targets the main landmark", () => {
    render(
      providers(
        <Routes>
          <Route element={<Shell />}>
            <Route index element={<div>today-screen</div>} />
          </Route>
        </Routes>,
      ),
    );

    // The skip link is the first focusable thing in the document.
    const skip = document.querySelector<HTMLAnchorElement>("a.skip-link");
    expect(skip).toBeTruthy();
    expect(skip?.getAttribute("href")).toBe("#main");
    expect(skip?.textContent?.trim().length).toBeGreaterThan(0);

    const main = document.getElementById("main");
    expect(main?.tagName).toBe("MAIN");
    // Focus goes to main when the skip link is followed, so it must accept it.
    expect(main?.getAttribute("tabindex")).toBe("-1");
  });

  it("keeps the seven destinations as real links in the tab order", () => {
    render(
      providers(
        <Routes>
          <Route element={<Shell />}>
            <Route index element={<div>today-screen</div>} />
          </Route>
        </Routes>,
      ),
    );
    for (const link of screen.getAllByRole("link")) {
      expect(link.tagName).toBe("A");
      expect(link.getAttribute("href")).toBeTruthy();
      expect(link.getAttribute("tabindex")).toBeNull();
    }
  });
});

describe("Kill Switch — one control, reachable from the keyboard", () => {
  it("is a native, focusable button that can be activated", async () => {
    render(providers(<KillSwitch />));

    const button = await screen.findByRole("button");
    expect(button.tagName).toBe("BUTTON");
    expect((button as HTMLButtonElement).disabled).toBe(false);
    expect(button.getAttribute("tabindex")).toBeNull();
    expect(button.getAttribute("aria-keyshortcuts")).toContain("Shift+K");

    button.focus();
    expect(document.activeElement).toBe(button);

    fireEvent.click(button);
    await waitFor(() => expect(commands.killSwitchSet).toHaveBeenCalledWith(true));
  });

  it("announces the engaged state as text", async () => {
    vi.mocked(commands.killSwitchStatus).mockReturnValue(
      ok({ engaged: true, since: "2026-10-17T00:00:00Z" }) as never,
    );
    render(providers(<KillSwitch />));

    const status = await screen.findByRole("status");
    expect(status.textContent?.trim().length).toBeGreaterThan(0);
  });
});

describe("Tracker — a card moves without a drag", () => {
  it("moves a card along the columns with Cmd/Ctrl + Arrow", async () => {
    render(providers(<Board />));

    // The card is a plain list item; its open control is the focusable button
    // and the key event bubbles to the card's Cmd/Ctrl + ←/→ handler (A11Y-2).
    const open = await screen.findByRole("button", { name: "Open Backend Engineer" });
    fireEvent.keyDown(open, { key: "ArrowRight", metaKey: true });

    await waitFor(() =>
      expect(commands.trackerTransitionStatus).toHaveBeenCalledWith(
        expect.objectContaining({ application_id: "app_1", to_status: "preparing" }),
      ),
    );
  });
});

describe("Wizard — focus starts inside the dialog", () => {
  it("takes focus on open, so the first Tab is already inside setup", async () => {
    render(providers(<OnboardingWizard />));
    const dialog = await screen.findByRole("dialog");
    expect(dialog.contains(document.activeElement)).toBe(true);
  });
});
