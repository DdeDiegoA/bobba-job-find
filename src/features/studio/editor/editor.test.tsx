import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { StudioDocument } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import StudioEditor, { type EditSlot } from "./index";

/** A CV as the core sends it: a summary, one section, one unbacked line. */
function cv(overrides: Partial<StudioDocument> = {}): StudioDocument {
  return {
    id: "doc_cv_2",
    kind: "cv",
    version: 2,
    lang: "en",
    template: "cv-classic",
    created_at: "2026-10-17T00:00:00Z",
    untrusted: true,
    language_fallback_from: null,
    text: {
      summary: {
        text: "Backend developer.",
        fact_ids: ["fact_1"],
        facts: [{ id: "fact_1", label: "Ada Lovelace" }],
        backed: true,
        unknown_facts: [],
      },
      sections: [
        {
          heading: "Experience",
          items: [
            {
              text: "Shipped a billing API.",
              fact_ids: ["fact_2"],
              facts: [{ id: "fact_2", label: "Shipped a billing API in Rust" }],
              backed: true,
              unknown_facts: [],
            },
            {
              text: "Led the platform team.",
              fact_ids: [],
              facts: [],
              backed: false,
              unknown_facts: [],
            },
          ],
        },
      ],
      paragraphs: [],
    },
    ready: false,
    checks: null,
    previous: { id: "doc_cv_1", version: 1 },
    diff: [
      {
        slot: "Experience 1",
        before: "Shipped a billing API.",
        after: "Shipped the billing API for 2M users.",
        change: "changed",
      },
    ],
    review: null,
    ...overrides,
  } as StudioDocument;
}

function cover(): StudioDocument {
  return {
    ...cv(),
    id: "doc_cover_1",
    kind: "cover_letter",
    text: {
      summary: null,
      sections: [],
      paragraphs: [
        {
          text: "I would like to apply.",
          fact_ids: ["fact_1"],
          facts: [{ id: "fact_1", label: "Ada Lovelace" }],
          backed: true,
          unknown_facts: [],
        },
        {
          text: "I built the billing API you describe.",
          fact_ids: ["fact_2"],
          facts: [{ id: "fact_2", label: "Shipped a billing API in Rust" }],
          backed: true,
          unknown_facts: [],
        },
      ],
    },
    previous: null,
    diff: [],
  } as StudioDocument;
}

describe("the draft editor (AC1, FR-79)", () => {
  it("shows every claim with the facts behind it", () => {
    render(<StudioEditor document={cv()} onEdit={vi.fn()} onRegenerate={vi.fn()} />);

    expect(screen.getByText("Backend developer.")).toBeTruthy();
    expect(screen.getByText("Experience")).toBeTruthy();
    expect(screen.getByText("Shipped a billing API.")).toBeTruthy();
    expect(screen.getByText(/Shipped a billing API in Rust/)).toBeTruthy();
    expect(screen.getByText("No Profile source")).toBeTruthy();
  });

  it("edits one claim inline and sends the slot the core addresses", () => {
    const onEdit = vi.fn<(slot: EditSlot, text: string) => void>();
    render(<StudioEditor document={cv()} onEdit={onEdit} onRegenerate={vi.fn()} />);

    const row = screen.getByText("Shipped a billing API.").closest("li") as HTMLElement;
    fireEvent.click(within(row).getByRole("button", { name: "Edit" }));

    const field = within(row).getByLabelText("The line's text");
    fireEvent.change(field, { target: { value: "Shipped the billing API for 2M users." } });
    fireEvent.click(within(row).getByRole("button", { name: "Save" }));

    expect(onEdit).toHaveBeenCalledWith(
      { slot: "cv_item", section: 0, item: 0 },
      "Shipped the billing API for 2M users.",
    );
  });

  it("never saves an empty line", () => {
    const onEdit = vi.fn();
    render(<StudioEditor document={cv()} onEdit={onEdit} onRegenerate={vi.fn()} />);

    const row = screen.getByText("Backend developer.").closest("li") as HTMLElement;
    fireEvent.click(within(row).getByRole("button", { name: "Edit" }));
    fireEvent.change(within(row).getByLabelText("The line's text"), { target: { value: " " } });
    fireEvent.click(within(row).getByRole("button", { name: "Save" }));

    expect(onEdit).not.toHaveBeenCalled();
  });

  it("regenerates a section by its index and the summary as its own slot", () => {
    const onRegenerate = vi.fn<(section: number | null) => void>();
    render(<StudioEditor document={cv()} onEdit={vi.fn()} onRegenerate={onRegenerate} />);

    fireEvent.click(screen.getByRole("button", { name: "Regenerate Experience" }));
    expect(onRegenerate).toHaveBeenCalledWith(0);

    fireEvent.click(screen.getByRole("button", { name: "Regenerate Summary" }));
    expect(onRegenerate).toHaveBeenCalledWith(null);
  });

  it("regenerates one cover-letter paragraph", () => {
    const onRegenerate = vi.fn<(section: number | null) => void>();
    render(<StudioEditor document={cover()} onEdit={vi.fn()} onRegenerate={onRegenerate} />);

    fireEvent.click(screen.getByRole("button", { name: "Regenerate Paragraph 2" }));
    expect(onRegenerate).toHaveBeenCalledWith(1);
  });

  it("toggles the diff against the previous draft", () => {
    render(<StudioEditor document={cv()} onEdit={vi.fn()} onRegenerate={vi.fn()} />);

    expect(screen.queryByText("Shipped the billing API for 2M users.")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Show diff" }));
    expect(screen.getByText("Shipped the billing API for 2M users.")).toBeTruthy();
    expect(screen.getByText("Changes since version 1")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Hide diff" }));
    expect(screen.queryByText("Shipped the billing API for 2M users.")).toBeNull();
  });

  it("shows what the fit cut and a heading left at the bottom of a page", () => {
    const document = cv({
      checks: {
        fabrication_ready: true,
        layout: { pages: 2, orphans: 1, page_budget: 2, passes: false },
        text_layer: {
          passed: true,
          missing_contact: [],
          missing_keywords: [],
          out_of_order: [],
          undecodable: [],
        },
        verified: false,
        cut: [{ section: "Experience", text: "Cut p99 latency by half.", score: 0.1 }],
      },
    });
    render(<StudioEditor document={document} onEdit={vi.fn()} onRegenerate={vi.fn()} />);

    expect(screen.getByText(/Cut p99 latency by half/)).toBeTruthy();
    expect(screen.getByText(/1 heading ends a page/)).toBeTruthy();
  });

  it("explains a draft in English for a posting in another language", () => {
    render(
      <StudioEditor
        document={cv({ language_fallback_from: "de" })}
        onEdit={vi.fn()}
        onRegenerate={vi.fn()}
      />,
    );
    expect(screen.getByText(/The posting is in de/)).toBeTruthy();
  });
});
