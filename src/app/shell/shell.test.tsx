import { fireEvent, render, screen, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { describe, expect, it } from "vitest";
import { ThemeProvider } from "@/app/theme";
import { NotFound, Shell } from "./index";
import type { RunState } from "./nav";
import { Topbar } from "./topbar";

const EXPECTED_NAV = [
  "/",
  "/inbox",
  "/studio",
  "/tracker",
  "/profile",
  "/connections",
  "/settings",
];

function renderShell(initialPath = "/") {
  return render(
    <ThemeProvider>
      <MemoryRouter initialEntries={[initialPath]}>
        <Routes>
          <Route element={<Shell />}>
            <Route index element={<div>today-screen</div>} />
            <Route path="inbox" element={<div>inbox-screen</div>} />
            <Route path="studio" element={<div>studio-screen</div>} />
            <Route path="tracker" element={<div>tracker-screen</div>} />
            <Route path="profile" element={<div>profile-screen</div>} />
            <Route path="connections" element={<div>connections-screen</div>} />
            <Route path="settings/*" element={<div>settings-screen</div>} />
            <Route path="*" element={<NotFound />} />
          </Route>
        </Routes>
      </MemoryRouter>
    </ThemeProvider>,
  );
}

describe("Shell — sidebar navigation (AC1)", () => {
  it("renders the seven primary destinations in order", () => {
    renderShell("/");
    const sidebar = screen.getByRole("complementary", { name: "Primary" });
    const hrefs = within(sidebar)
      .getAllByRole("link")
      .map((link) => link.getAttribute("href"));
    expect(hrefs).toEqual(EXPECTED_NAV);
  });

  it("marks the active destination and labels it for screen readers", () => {
    renderShell("/studio");
    const sidebar = screen.getByRole("complementary", { name: "Primary" });
    const studio = within(sidebar).getByRole("link", { name: "Studio" });
    expect(studio.getAttribute("aria-current")).toBe("page");
  });
});

describe("Shell — topbar (AC1)", () => {
  it("shows the command-palette trigger, Run now and the run status", () => {
    renderShell("/");
    expect(screen.getByRole("button", { name: "Search" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Run now" })).toBeTruthy();
    expect(screen.getByText("No runs yet")).toBeTruthy();
  });

  it("reflects the current route in the title", () => {
    renderShell("/connections");
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Connections");
  });

  it("shows the Kill Switch only when Auto is on", () => {
    const ref = { current: null };
    const idle: RunState = { running: false, autoEnabled: false, lastRunAt: null, newCount: 0 };
    const autoOn: RunState = { ...idle, autoEnabled: true };

    const { unmount } = render(
      <MemoryRouter>
        <Topbar run={idle} searchRef={ref} onSearch={() => {}} />
      </MemoryRouter>,
    );
    expect(document.querySelector('[data-slot="kill-switch"]')).toBeNull();
    unmount();

    render(
      <MemoryRouter>
        <Topbar run={autoOn} searchRef={ref} onSearch={() => {}} />
      </MemoryRouter>,
    );
    expect(document.querySelector('[data-slot="kill-switch"]')).not.toBeNull();
  });

  it("disables Run now while a run is in progress", () => {
    const ref = { current: null };
    render(
      <MemoryRouter>
        <Topbar
          run={{ running: true, autoEnabled: false, lastRunAt: null, newCount: 0 }}
          searchRef={ref}
          onSearch={() => {}}
        />
      </MemoryRouter>,
    );
    const button = screen.getByRole("button", { name: "Running…" }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
  });

  it("interpolates the run status through ICU", () => {
    const ref = { current: null };
    render(
      <MemoryRouter>
        <Topbar
          run={{
            running: false,
            autoEnabled: false,
            lastRunAt: "2026-10-05T06:12:00Z",
            newCount: 12,
          }}
          searchRef={ref}
          onSearch={() => {}}
        />
      </MemoryRouter>,
    );
    expect(screen.getByText(/Last run .*12 new/)).toBeTruthy();
  });
});

describe("Shell — keyboard shortcuts (AC1/EXPERIENCE §6)", () => {
  it("Cmd/Ctrl+K focuses the search trigger", () => {
    renderShell("/");
    fireEvent.keyDown(window, { key: "k", metaKey: true });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Search" }));
  });

  it("Cmd/Ctrl+digit jumps to the matching destination", () => {
    renderShell("/");
    fireEvent.keyDown(window, { key: "3", metaKey: true });
    expect(screen.getByText("studio-screen")).toBeTruthy();
  });

  it("g-sequences navigate mouselessly", () => {
    renderShell("/");
    fireEvent.keyDown(window, { key: "g" });
    fireEvent.keyDown(window, { key: "i" });
    expect(screen.getByText("inbox-screen")).toBeTruthy();
  });
});

describe("Shell — accessibility and empty states", () => {
  it("provides a skip link to the main region", () => {
    renderShell("/");
    const skip = screen.getByRole("link", { name: "Skip to content" });
    expect(skip.getAttribute("href")).toBe("#main");
    expect(document.getElementById("main")).not.toBeNull();
  });

  it("renders a placeholder empty state for unknown routes", () => {
    renderShell("/does-not-exist");
    expect(screen.getByText("Nothing here yet")).toBeTruthy();
  });

  it("exposes a labelled theme control and language toggle", () => {
    renderShell("/");
    expect(screen.getByRole("button", { name: /Theme: System/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Language: EN/ })).toBeTruthy();
  });
});
