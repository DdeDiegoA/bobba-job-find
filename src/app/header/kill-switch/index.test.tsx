import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Engagement, KillSwitchChanged, KillSwitchView } from "@/lib/ipc/bindings";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import KillSwitch from "./index";

/** The callback the component registered for `kill_switch:changed`. */
let onChanged: ((event: { payload: KillSwitchChanged }) => void) | null = null;

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    killSwitchStatus: vi.fn(),
    killSwitchSet: vi.fn(),
  },
  events: {
    killSwitchChanged: {
      listen: vi.fn((cb: (event: { payload: KillSwitchChanged }) => void) => {
        onChanged = cb;
        return Promise.resolve(() => {
          onChanged = null;
        });
      }),
    },
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const off: KillSwitchView = { engaged: false, since: null };
const on: KillSwitchView = { engaged: true, since: "2026-10-17T10:00:00Z" };

const engagement = (view: KillSwitchView): Engagement => ({
  view,
  parked: 0,
  cancelled_runs: 0,
  browser_closed: false,
});

beforeEach(() => {
  vi.mocked(commands.killSwitchStatus).mockReset();
  vi.mocked(commands.killSwitchSet).mockReset();
  onChanged = null;
  vi.mocked(commands.killSwitchStatus).mockImplementation(() => ok(off));
  vi.mocked(commands.killSwitchSet).mockImplementation((engaged) =>
    ok(engagement(engaged ? on : off)),
  );
});

describe("Kill Switch (FR-86)", () => {
  it("engages from the header and shows the quiet chip", async () => {
    render(<KillSwitch />);
    const engage = await screen.findByRole("button", { name: "Kill Switch" });
    expect(engage.getAttribute("aria-keyshortcuts")).toBe("Meta+Shift+K Control+Shift+K");

    fireEvent.click(engage);

    await waitFor(() => expect(commands.killSwitchSet).toHaveBeenCalledWith(true));
    expect(await screen.findByText("Automatic sending is off")).toBeTruthy();
  });

  it("starts on the chip when the switch is already engaged", async () => {
    vi.mocked(commands.killSwitchStatus).mockImplementation(() => ok(on));
    render(<KillSwitch />);

    expect(await screen.findByText("Automatic sending is off")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Kill Switch" })).toBeNull();
  });

  it("turns it off only from the chip's explicit control", async () => {
    vi.mocked(commands.killSwitchStatus).mockImplementation(() => ok(on));
    render(<KillSwitch />);

    fireEvent.click(await screen.findByRole("button", { name: "Turn off" }));

    await waitFor(() => expect(commands.killSwitchSet).toHaveBeenCalledWith(false));
    expect(await screen.findByRole("button", { name: "Kill Switch" })).toBeTruthy();
  });

  it("follows a switch thrown from the tray or the shortcut", async () => {
    render(<KillSwitch />);
    await screen.findByRole("button", { name: "Kill Switch" });

    act(() => {
      onChanged?.({ payload: { engaged: true, since: "2026-10-17T10:00:00Z" } });
    });

    expect(await screen.findByText("Automatic sending is off")).toBeTruthy();
  });
});
