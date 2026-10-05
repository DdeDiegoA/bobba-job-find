import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";

export type Theme = "system" | "light" | "dark";

/** Persisted manual override (AC2). The settings table (S1.7) may own this later. */
const STORAGE_KEY = "boba.theme";

function systemPrefersDark(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return false;
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

function readStoredTheme(): Theme {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    return value === "light" || value === "dark" || value === "system" ? value : "system";
  } catch {
    return "system";
  }
}

function storeTheme(theme: Theme): void {
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // Storage disabled (e.g. hardening): the choice still applies for this run.
  }
}

interface ThemeContextValue {
  /** The chosen mode: follow the OS, or a persisted manual override. */
  theme: Theme;
  /** Whether the effective theme is dark (resolved through the OS). */
  isDark: boolean;
  setTheme: (theme: Theme) => void;
  /** `Cmd/Ctrl+Shift+L`: override to the opposite of the current effective theme. */
  toggleTheme: () => void;
  /** Cycle system → light → dark → system (the sidebar fallback control). */
  cycleTheme: () => void;
}

const ThemeContext = createContext<ThemeContextValue>({
  theme: "system",
  isDark: false,
  setTheme: () => undefined,
  toggleTheme: () => undefined,
  cycleTheme: () => undefined,
});

/** Light / dark / follow-OS with a persisted manual override (owned by S1.5). */
export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(readStoredTheme);
  const [osDark, setOsDark] = useState<boolean>(systemPrefersDark);

  // Follow the OS while the mode is `system` (AC2).
  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => setOsDark(media.matches);
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, []);

  const isDark = theme === "dark" || (theme === "system" && osDark);

  useEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dark", isDark);
    root.style.colorScheme = isDark ? "dark" : "light";
  }, [isDark]);

  const setTheme = useCallback((next: Theme) => {
    setThemeState(next);
    storeTheme(next);
  }, []);

  const toggleTheme = useCallback(() => {
    setThemeState((prev) => {
      const dark = prev === "dark" || (prev === "system" && systemPrefersDark());
      const next: Theme = dark ? "light" : "dark";
      storeTheme(next);
      return next;
    });
  }, []);

  const cycleTheme = useCallback(() => {
    setThemeState((prev) => {
      const next: Theme = prev === "system" ? "light" : prev === "light" ? "dark" : "system";
      storeTheme(next);
      return next;
    });
  }, []);

  const value = useMemo(
    () => ({ theme, isDark, setTheme, toggleTheme, cycleTheme }),
    [theme, isDark, setTheme, toggleTheme, cycleTheme],
  );

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export const useTheme = () => useContext(ThemeContext);
