import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { ThemeProvider, useTheme } from "./index";

function setOsPrefersDark(initial: boolean) {
  const listeners = new Set<(event: MediaQueryListEvent) => void>();
  const mql = {
    matches: initial,
    media: "(prefers-color-scheme: dark)",
    onchange: null,
    addEventListener: (_type: string, cb: (event: MediaQueryListEvent) => void) =>
      listeners.add(cb),
    removeEventListener: (_type: string, cb: (event: MediaQueryListEvent) => void) =>
      listeners.delete(cb),
    dispatchEvent: () => false,
  } as unknown as MediaQueryList;

  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: () => mql,
  });

  return {
    flip(matches: boolean) {
      (mql as { matches: boolean }).matches = matches;
      for (const cb of listeners) cb({ matches } as MediaQueryListEvent);
    },
  };
}

function Probe() {
  const { theme, isDark, toggleTheme, setTheme, cycleTheme } = useTheme();
  return (
    <div>
      <span data-testid="theme">{theme}</span>
      <span data-testid="dark">{String(isDark)}</span>
      <button type="button" onClick={toggleTheme}>
        toggle
      </button>
      <button type="button" onClick={() => setTheme("light")}>
        set-light
      </button>
      <button type="button" onClick={cycleTheme}>
        cycle
      </button>
    </div>
  );
}

function renderProbe() {
  return render(
    <ThemeProvider>
      <Probe />
    </ThemeProvider>,
  );
}

afterEach(() => {
  document.documentElement.classList.remove("dark");
  document.documentElement.style.colorScheme = "";
  localStorage.clear();
});

describe("ThemeProvider (AC2)", () => {
  it("follows the OS into dark on load", () => {
    setOsPrefersDark(true);
    renderProbe();
    expect(screen.getByTestId("dark").textContent).toBe("true");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("follows the OS into light on load", () => {
    setOsPrefersDark(false);
    renderProbe();
    expect(screen.getByTestId("dark").textContent).toBe("false");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("reacts to an OS light/dark switch while on System", () => {
    const os = setOsPrefersDark(false);
    renderProbe();
    act(() => os.flip(true));
    expect(screen.getByTestId("dark").textContent).toBe("true");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("restores a persisted manual override", () => {
    setOsPrefersDark(false);
    localStorage.setItem("boba.theme", "dark");
    renderProbe();
    expect(screen.getByTestId("theme").textContent).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("toggles to an explicit override and persists it", () => {
    setOsPrefersDark(false);
    renderProbe();
    fireEvent.click(screen.getByRole("button", { name: "toggle" }));
    expect(screen.getByTestId("theme").textContent).toBe("dark");
    expect(localStorage.getItem("boba.theme")).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("cycles system → light → dark → system and persists each step", () => {
    setOsPrefersDark(true);
    renderProbe();
    const cycle = screen.getByRole("button", { name: "cycle" });
    fireEvent.click(cycle);
    expect(screen.getByTestId("theme").textContent).toBe("light");
    fireEvent.click(cycle);
    expect(screen.getByTestId("theme").textContent).toBe("dark");
    fireEvent.click(cycle);
    expect(screen.getByTestId("theme").textContent).toBe("system");
    expect(localStorage.getItem("boba.theme")).toBe("system");
  });
});
