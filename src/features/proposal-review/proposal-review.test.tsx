import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ProposalView } from "@/lib/ipc/bindings";
import { ProposalReviewCard, type ProposalReviewLabels } from "./index";

/**
 * The shared review card (FX8): every AI proposal is shown verbatim and as
 * plain text, and nothing leaves the card until the user confirms or edits it.
 */

const proposal = (over: Partial<ProposalView> = {}): ProposalView => ({
  id: "prop_1",
  item_id: null,
  kind: "experience",
  text_en: "Senior Backend Engineer",
  text_es: "Ingeniero Backend Senior",
  evidence: null,
  detail_json: "{}",
  source_excerpt: null,
  untrusted_origin: false,
  created_at: "2026-10-18T09:00:00.000Z",
  ...over,
});

const labels: ProposalReviewLabels = {
  kindLabel: "Experience",
  textEn: "English",
  textEs: "Spanish",
  confirm: "Confirm",
  edit: "Edit",
  saveEdit: "Save edit",
  cancel: "Cancel",
  discard: "Delete",
  editHint: "This is the full proposed text. Edit the wording before saving.",
  untrusted: "Boba read this from text it didn't write. Check it before saving.",
};

function renderCard(over: Partial<Parameters<typeof ProposalReviewCard>[0]> = {}) {
  const props = {
    proposal: proposal(),
    busy: false,
    labels,
    onConfirm: vi.fn(),
    onSaveEdit: vi.fn(),
    onDiscard: vi.fn(),
    ...over,
  };
  render(<ProposalReviewCard {...props} />);
  return props;
}

describe("ProposalReviewCard — verbatim, confirm-only", () => {
  it("shows the proposed text verbatim as plain text", () => {
    const long = "Line one\nLine two — the user's own words, kept exactly as written.";
    renderCard({ proposal: proposal({ text_en: long }) });

    const text = screen.getByText(long, { normalizer: (value) => value });
    expect(text.tagName).toBe("P");
    expect(text.className).toContain("whitespace-pre-wrap");
  });

  it("saves nothing when rendered or when the user only reads it", () => {
    const props = renderCard();
    expect(props.onConfirm).not.toHaveBeenCalled();
    expect(props.onSaveEdit).not.toHaveBeenCalled();
    expect(props.onDiscard).not.toHaveBeenCalled();
  });

  it("confirms the proposal as proposed", () => {
    const props = renderCard();
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    expect(props.onConfirm).toHaveBeenCalledTimes(1);
  });

  it("sends the user's edit, with the proposal's evidence and detail", () => {
    const props = renderCard({
      proposal: proposal({ evidence: "from the CV", detail_json: '{"org":"Acme"}' }),
    });
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText("Spanish"), {
      target: { value: "Ingeniero Senior" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save edit" }));

    expect(props.onSaveEdit).toHaveBeenCalledWith({
      text_en: "Senior Backend Engineer",
      text_es: "Ingeniero Senior",
      evidence: "from the CV",
      detail_json: '{"org":"Acme"}',
    });
  });

  it("discards without saving", () => {
    const props = renderCard();
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(props.onDiscard).toHaveBeenCalledTimes(1);
    expect(props.onConfirm).not.toHaveBeenCalled();
  });

  it("warns when the proposal came from third-party text", () => {
    renderCard({ proposal: proposal({ untrusted_origin: true }) });
    expect(
      screen.getByText("Boba read this from text it didn't write. Check it before saving."),
    ).toBeInTheDocument();
  });
});
