import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import "@/lib/i18n";
import StudioTrace, { type TraceSentence } from "./index";

const sentence = (over: Partial<TraceSentence>): TraceSentence => ({
  text: "Shipped an API",
  facts: [],
  ...over,
});

/** The fact the Profile backs the first test sentence with. */
const FACT = { id: "fact_1", label: "Senior Backend Engineer at Nimbus" };

function renderPanel(sentences: TraceSentence[]) {
  const onFix = vi.fn();
  render(<StudioTrace sentences={sentences} onFix={onFix} />);
  return onFix;
}

const row = (text: string) =>
  screen.getByRole("button", { name: text }).closest("li") as HTMLElement;

describe("claim traceability (AC2)", () => {
  it("shows the Profile facts behind a sentence when it is focused", () => {
    renderPanel([sentence({ text: "Shipped an API", facts: [FACT] })]);

    const trigger = screen.getByRole("button", { name: "Shipped an API" });
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText(FACT.label)).toBeNull();

    fireEvent.focus(trigger);

    expect(screen.getByText(FACT.label)).toBeTruthy();
    expect(screen.getByText("From your Profile")).toBeTruthy();
    expect(trigger.getAttribute("aria-expanded")).toBe("true");
    expect(row("Shipped an API").dataset.trace).toBe("backed");
  });

  it("shows them on hover too, and takes them away when the pointer leaves", () => {
    renderPanel([sentence({ text: "Shipped an API", facts: [FACT] })]);

    const trigger = screen.getByRole("button", { name: "Shipped an API" });
    fireEvent.mouseOver(trigger);
    expect(screen.getByText(FACT.label)).toBeTruthy();

    fireEvent.mouseOut(trigger);
    expect(screen.queryByText(FACT.label)).toBeNull();
  });

  it("highlights a sentence that nothing in the Profile backs", () => {
    renderPanel([sentence({ text: "Results-driven engineer.", facts: [] })]);

    const unbacked = row("Results-driven engineer.");
    expect(unbacked.dataset.trace).toBe("unbacked");
    expect(within(unbacked).getByText("No Profile source")).toBeTruthy();
    // The panel does not claim a source that does not exist.
    fireEvent.focus(screen.getByRole("button", { name: "Results-driven engineer." }));
    expect(screen.queryByText("From your Profile")).toBeNull();
  });
});

describe("the no-fabrication gate (AC1)", () => {
  it("names the fact ids the Profile does not have", () => {
    renderPanel([
      sentence({ text: "Kubernetes and Terraform", facts: [FACT], unknown: ["fact_ghost"] }),
    ]);

    fireEvent.focus(screen.getByRole("button", { name: "Kubernetes and Terraform" }));

    const missing = screen.getByText(/Not in your Profile/);
    expect(missing.textContent).toContain("fact_ghost");
    expect(row("Kubernetes and Terraform").dataset.trace).toBe("unbacked");
  });

  it("says Ready and Send stay blocked while a sentence needs fixing", () => {
    renderPanel([
      sentence({ text: "Shipped an API", facts: [FACT] }),
      sentence({ text: "Nothing backs this.", facts: [] }),
    ]);

    const status = screen.getByRole("status");
    expect(status.textContent).toContain("1 sentence needs attention");
    expect(status.textContent).toContain("Ready and Send stay blocked");
  });

  it("says the draft is ready when every sentence is backed", () => {
    renderPanel([
      sentence({ text: "Shipped an API", facts: [FACT] }),
      sentence({
        text: "Led a team of four",
        facts: [{ id: "fact_2", label: "Team lead at Nimbus" }],
      }),
    ]);

    expect(screen.getByRole("status").textContent).toContain("Every sentence is backed");
    expect(screen.queryByRole("button", { name: "Fix" })).toBeNull();
    expect(row("Shipped an API").dataset.trace).toBe("backed");
  });

  it("fixes the exact sentence through the callback", () => {
    const broken = sentence({ text: "Nothing backs this.", facts: [] });
    const onFix = renderPanel([sentence({ text: "Shipped an API", facts: [FACT] }), broken]);

    fireEvent.click(within(row("Nothing backs this.")).getByRole("button", { name: "Fix" }));

    expect(onFix).toHaveBeenCalledWith(broken);
  });
});
