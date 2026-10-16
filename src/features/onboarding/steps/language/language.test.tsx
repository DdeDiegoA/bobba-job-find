import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { ThemeProvider } from "@/app/theme";
import { currentLocale, setLocale } from "@/lib/i18n";
import LanguageStep from "./index";

/** Stands in for the wizard: keeps the step's answers like the real container. */
function Harness({ draft = {} }: { draft?: Record<string, unknown> }) {
  const [answers, setAnswers] = useState(draft);
  return (
    <ThemeProvider>
      <LanguageStep
        answers={answers}
        setAnswer={(key, value) => setAnswers((a) => ({ ...a, [key]: value }))}
      />
    </ThemeProvider>
  );
}

const radio = (name: string) => screen.getByRole("radio", { name }) as HTMLInputElement;

afterEach(() => {
  setLocale("en");
  document.documentElement.classList.remove("dark");
  document.documentElement.style.colorScheme = "";
  localStorage.clear();
});

describe("Language & theme step (FR-1, AC1)", () => {
  it("starts on the current UI language", () => {
    render(<Harness />);
    expect(currentLocale()).toBe("en");
    expect(radio("English").checked).toBe(true);
    expect(radio("Español").checked).toBe(false);
  });

  it("switches the UI language live and remembers it", () => {
    render(<Harness />);
    fireEvent.click(radio("Español"));
    expect(currentLocale()).toBe("es");
    expect(localStorage.getItem("boba.locale")).toBe("es");
    expect(radio("Español").checked).toBe(true);
  });

  it("applies the chosen theme", () => {
    render(<Harness />);
    fireEvent.click(radio("Dark"));
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(localStorage.getItem("boba.theme")).toBe("dark");
  });

  it("re-applies a draft from a previous session (AC2)", () => {
    render(<Harness draft={{ locale: "es", theme: "dark" }} />);
    // The draft's language applies live, so the step itself is already in Spanish.
    expect(currentLocale()).toBe("es");
    expect(radio("Español").checked).toBe(true);
    expect(radio("Oscuro").checked).toBe(true);
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });
});
