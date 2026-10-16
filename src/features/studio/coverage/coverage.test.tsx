import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import "@/lib/i18n";
import StudioCoverage, { type CoverageRequirement } from "./index";

/** The UJ-3 fixture: 12 of 14 keywords covered, "terraform" and "gcp" gaps. */
const REQUIREMENTS: CoverageRequirement[] = [
  { requirement: "Rust and Postgres", covered: ["rust", "postgres"], gaps: [] },
  { requirement: "Data modelling", covered: ["modelling"], gaps: [] },
  { requirement: "Terraform", covered: [], gaps: ["terraform"] },
  { requirement: "GCP", covered: [], gaps: ["gcp"] },
];

const row = (requirement: string) =>
  screen.getByText(requirement).closest("li") as HTMLElement;

describe("keyword coverage and declared gaps (FR-75, AC2)", () => {
  it("shows every requirement as covered or a gap", () => {
    render(<StudioCoverage requirements={REQUIREMENTS} />);

    expect(row("Rust and Postgres").dataset.coverage).toBe("covered");
    expect(row("Terraform").dataset.coverage).toBe("gap");
    expect(within(row("Terraform")).getByText("Gap")).toBeTruthy();
    expect(within(row("Rust and Postgres")).getByText("Covered")).toBeTruthy();
  });

  it("counts the keywords the Profile backs out of the ones the Offer names", () => {
    render(<StudioCoverage requirements={REQUIREMENTS} />);

    expect(screen.getByText("Coverage 3/5")).toBeTruthy();
    expect(screen.getByRole("status").textContent).toContain("2 keywords have no Profile source");
  });

  it("names a gap's keywords instead of claiming them", () => {
    render(<StudioCoverage requirements={REQUIREMENTS} />);

    expect(
      within(row("Terraform")).getByText("No Profile source for: terraform").textContent,
    ).toBeTruthy();
    // A covered requirement lists what backs it, not a gap.
    expect(within(row("Data modelling")).queryByText(/No Profile source/)).toBeNull();
  });

  it("says the gaps were not added, and never writes them into the draft", () => {
    render(<StudioCoverage requirements={REQUIREMENTS} />);

    const panel = screen.getByRole("region", { name: "Keyword coverage" });
    const notAdded = panel.querySelector("[data-not-inserted]");
    expect(notAdded?.textContent).toContain("Not added: terraform, gcp");
    expect(notAdded?.textContent).toContain("never writes a keyword your Profile does not back");
    expect(panel.dataset.gaps).toBe("2");
  });

  it("says so when the Profile backs everything the Offer asks for", () => {
    render(
      <StudioCoverage
        requirements={[{ requirement: "Rust", covered: ["rust"], gaps: [] }]}
      />,
    );

    expect(screen.getByText("Coverage 1/1")).toBeTruthy();
    expect(screen.getByRole("status").textContent).toContain(
      "Your Profile backs every keyword the Offer asks for",
    );
    expect(screen.queryByText("Gap")).toBeNull();
    expect(screen.queryByText(/Not added/)).toBeNull();
  });

  // Cold start: an Offer whose requirements we have not read yet is not a crash,
  // and it never claims coverage it does not have.
  it("shows nothing covered when the Offer states no requirement", () => {
    render(<StudioCoverage requirements={[]} />);

    expect(screen.getByText("Coverage 0/0")).toBeTruthy();
    expect(screen.queryByText(/Not added/)).toBeNull();
  });
});
