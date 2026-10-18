import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { usePaletteStore } from "@/app/palette/store";
import { ThemeProvider } from "@/app/theme";
import "@/lib/i18n";
import {
  commands,
  type KillSwitchChanged,
  type RunReportProgressed,
  type RunReportView,
} from "@/lib/ipc/bindings";
import { NotFound, Shell } from "./index";
import { Topbar } from "./topbar";
import type { RunState } from "./use-run-now";

/** The callbacks the components registered for the two live event streams. */
const listeners = vi.hoisted(() => ({
  run: [] as ((event: { payload: RunReportProgressed }) => void)[],
  kill: [] as ((event: { payload: KillSwitchChanged }) => void)[],
}));

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    jobsRunNow: vi.fn(),
    runReportLatest: vi.fn(),
    killSwitchStatus: vi.fn(),
    killSwitchSet: vi.fn(),
    offersInbox: vi.fn(),
  },
  events: {
    runReportProgressed: {
      listen: vi.fn((cb: (event: { payload: RunReportProgressed }) => void) => {
        listeners.run.push(cb);
        return Promise.resolve(() => {
          const index = listeners.run.indexOf(cb);
          if (index >= 0) listeners.run.splice(index, 1);
        });
      }),
    },
    killSwitchChanged: {
      listen: vi.fn((cb: (event: { payload: KillSwitchChanged }) => void) => {
        listeners.kill.push(cb);
        return Promise.resolve(() => {
          const index = listeners.kill.indexOf(cb);
          if (index >= 0) listeners.kill.splice(index, 1);
        });
      }),
    },
  },
}));

const EXPECTED_NAV = [
  "/",
  "/inbox",
  "/studio",
  "/tracker",
  "/profile",
  "/connections",
  "/settings",
];

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

function report(overrides: Partial<RunReportView> = {}): RunReportView {
  return {
    run_id: "run-1",
    retry_of: null,
    started_at: "2026-10-18T09:00:00Z",
    finished_at: "2026-10-18T09:05:00Z",
    duration_ms: 300_000,
    sources: [],
    queried: 1,
    fetched: 1,
    new: 0,
    duplicates: 0,
    vetoed: 0,
    ranked: 1,
    agent_tasks_used: 0,
    errors: [],
    partial: false,
    ...overrides,
  };
}

function progress(phase: string): RunReportProgressed {
  return {
    run_id: "run-1",
    phase,
    done: phase === "finished" ? 1 : 0,
    total: 1,
    source_id: null,
    source_name: null,
    outcome: null,
    partial: null,
  };
}

/** Deliver an event to every listener currently registered (the run hooks). */
function emitRunProgress(payload: RunReportProgressed) {
  act(() => {
    for (const cb of [...listeners.run]) cb({ payload });
  });
}

beforeEach(() => {
  // S6.7: the palette is store-driven, so every test starts with it closed.
  usePaletteStore.setState({ open: false });
  listeners.run.length = 0;
  listeners.kill.length = 0;
  vi.mocked(commands.jobsRunNow).mockReset();
  vi.mocked(commands.runReportLatest).mockReset();
  vi.mocked(commands.killSwitchStatus).mockReset();
  vi.mocked(commands.killSwitchSet).mockReset();
  vi.mocked(commands.offersInbox).mockReset();
  vi.mocked(commands.jobsRunNow).mockImplementation(() => ok({ run_id: "run-1", joined: false }));
  vi.mocked(commands.runReportLatest).mockImplementation(() => ok(null));
  vi.mocked(commands.killSwitchStatus).mockImplementation(() =>
    ok({ engaged: false, since: null }),
  );
  vi.mocked(commands.offersInbox).mockImplementation(() => ok([]));
});

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

  it("disables Run now while a run is in progress", () => {
    const ref = { current: null };
    const running: RunState = { running: true, lastRunAt: null, newCount: 0 };
    render(
      <MemoryRouter>
        <Topbar
          run={running}
          message={null}
          pending={false}
          onRunNow={() => {}}
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
            lastRunAt: "2026-10-05T06:12:00Z",
            newCount: 12,
          }}
          message={null}
          pending={false}
          onRunNow={() => {}}
          searchRef={ref}
          onSearch={() => {}}
        />
      </MemoryRouter>,
    );
    expect(screen.getByText(/Last run .*12 new/)).toBeTruthy();
  });

  it("announces a run status message to assistive tech", () => {
    const ref = { current: null };
    render(
      <MemoryRouter>
        <Topbar
          run={{ running: false, lastRunAt: null, newCount: 0 }}
          message={{ kind: "error", text: "The store is locked." }}
          pending={false}
          onRunNow={() => {}}
          searchRef={ref}
          onSearch={() => {}}
        />
      </MemoryRouter>,
    );
    expect(screen.getByRole("alert").textContent).toBe("The store is locked.");
  });
});

describe("Shell — Run now (FX4)", () => {
  it("calls jobs_run_now once and shows the running state (AC1)", async () => {
    renderShell("/");
    fireEvent.click(screen.getByRole("button", { name: "Run now" }));

    await waitFor(() => expect(commands.jobsRunNow).toHaveBeenCalledTimes(1));
    const running = await screen.findByRole("button", { name: "Running…" });
    expect((running as HTMLButtonElement).disabled).toBe(true);
    const sidebar = screen.getByRole("complementary", { name: "Primary" });
    expect(within(sidebar).getByText("Running…")).toBeTruthy();
  });

  it("says a run is already in progress when the core joined it (AC2)", async () => {
    vi.mocked(commands.jobsRunNow).mockImplementation(() => ok({ run_id: "run-1", joined: true }));
    renderShell("/");
    fireEvent.click(screen.getByRole("button", { name: "Run now" }));

    expect(await screen.findByText("A run is already in progress")).toBeTruthy();
    expect(commands.jobsRunNow).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Running…" })).toBeTruthy();
  });

  it("shows the translated core error when the store is locked (AC3)", async () => {
    vi.mocked(commands.jobsRunNow).mockImplementation(() =>
      Promise.resolve({
        status: "error" as const,
        error: { code: "locked", message_key: "errors.store.locked" },
      }),
    );
    renderShell("/");
    fireEvent.click(screen.getByRole("button", { name: "Run now" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe("Boba can't reach your data yet. Unlock it to continue.");
    expect(screen.getByRole("button", { name: "Run now" })).toBeTruthy();
  });

  it("follows the run lifecycle events and updates the last run (AC4)", async () => {
    renderShell("/");
    emitRunProgress(progress("started"));
    expect(await screen.findByRole("button", { name: "Running…" })).toBeTruthy();

    vi.mocked(commands.runReportLatest).mockImplementation(() =>
      ok(report({ finished_at: "2026-10-18T09:05:00Z", new: 7 })),
    );
    emitRunProgress(progress("finished"));

    expect(await screen.findByText(/Last run .*7 new/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Run now" })).toBeTruthy();
  });

  it("shows the backup reason before a run can start (AC3)", async () => {
    vi.mocked(commands.jobsRunNow).mockImplementation(() =>
      Promise.resolve({
        status: "error" as const,
        error: { code: "conflict", message_key: "errors.backup.recovery_required" },
      }),
    );
    renderShell("/");
    fireEvent.click(screen.getByRole("button", { name: "Run now" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe("Create a recovery backup before a search run can start.");
  });

  it("stays on Run now when the run finishes before the command resolves (review Bug 1)", async () => {
    let resolveRun: (result: { status: "ok"; data: { run_id: string; joined: boolean } }) => void =
      () => {};
    vi.mocked(commands.jobsRunNow).mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveRun = resolve;
        }),
    );
    renderShell("/");
    fireEvent.click(screen.getByRole("button", { name: "Run now" }));
    // The command is in flight, and the run it started ends before it answers.
    await waitFor(() => expect(commands.jobsRunNow).toHaveBeenCalledTimes(1));
    emitRunProgress(progress("started"));
    emitRunProgress(progress("finished"));
    await act(async () => {
      resolveRun({ status: "ok", data: { run_id: "run-1", joined: false } });
    });

    expect(screen.getByRole("button", { name: "Run now" })).toBeTruthy();
  });

  it("surfaces a failed last-run read instead of saying there are no runs (review Low 2)", async () => {
    vi.mocked(commands.runReportLatest).mockImplementation(() =>
      Promise.resolve({
        status: "error" as const,
        error: { code: "locked", message_key: "errors.store.locked" },
      }),
    );
    renderShell("/");

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe("Boba can't reach your data yet. Unlock it to continue.");
  });

  it("renders the Kill Switch whether or not Auto is on (AC5)", async () => {
    renderShell("/");
    expect(await screen.findByRole("button", { name: "Kill Switch" })).toBeTruthy();
  });
});

describe("Shell — keyboard shortcuts (AC1/EXPERIENCE §6)", () => {
  it("Cmd/Ctrl+K opens the command palette (S6.7)", () => {
    renderShell("/");
    fireEvent.keyDown(window, { key: "k", metaKey: true });
    // The trigger's job since S6.7 is to open the palette, which takes the focus.
    expect(screen.getByRole("dialog", { name: "Command palette" })).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByRole("combobox"));
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
