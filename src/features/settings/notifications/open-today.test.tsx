import { act, render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { NotificationOpened } from "@/lib/ipc/bindings";
import { OpenToday } from "./open-today";

/** The callback the component registered for `notification:opened`. */
let onOpened: ((event: { payload: NotificationOpened }) => void) | null = null;

vi.mock("@/lib/ipc/bindings", () => ({
  events: {
    notificationOpened: {
      listen: vi.fn((cb: (event: { payload: NotificationOpened }) => void) => {
        onOpened = cb;
        return Promise.resolve(() => {
          onOpened = null;
        });
      }),
    },
  },
}));

function Here() {
  const location = useLocation();
  return <p data-testid="path">{location.pathname}</p>;
}

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <OpenToday />
      <Routes>
        <Route path="/" element={<Here />} />
        <Route path="/settings" element={<Here />} />
      </Routes>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  onOpened = null;
});

describe("A digest click opens Today (S5.3 AC1)", () => {
  it("routes to Today from wherever the user was", async () => {
    renderAt("/settings");
    expect((await screen.findByTestId("path")).textContent).toBe("/settings");
    await act(async () => {});

    act(() => onOpened?.({ payload: { target: "today" } }));

    expect(screen.getByTestId("path").textContent).toBe("/");
  });

  it("ignores a target it does not know", async () => {
    renderAt("/settings");
    await act(async () => {});

    act(() => onOpened?.({ payload: { target: "elsewhere" } }));

    expect(screen.getByTestId("path").textContent).toBe("/settings");
  });
});
