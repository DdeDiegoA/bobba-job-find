import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type ImportOutcome, type ProposalView } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import CvImportStep from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    profileImportStart: vi.fn(),
    profileImportProposals: vi.fn(),
    profileImportConfirm: vi.fn(),
    profileImportDiscard: vi.fn(),
  },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const proposal = (over: Partial<ProposalView> = {}): ProposalView => ({
  id: "prop_1",
  kind: "experience",
  text_en: "Backend engineer at Nimbus since 2020",
  text_es: null,
  evidence: null,
  detail_json: "{}",
  source_excerpt: "Backend Engineer — Nimbus, 2020–present",
  untrusted_origin: false,
  created_at: "2026-10-17T09:00:00.000Z",
  ...over,
});

const outcome = (proposals: ProposalView[], over: Partial<ImportOutcome> = {}): ImportOutcome => ({
  proposals,
  accepted: proposals.length,
  dropped: 0,
  files: ["cv.pdf"],
  ...over,
});

const snapshot = {
  profile_version: 1,
  facts: [],
  pending_proposals: 0,
};

/** Stands in for the wizard: keeps answers and shows them, like `StepProps`. */
function Harness({ draft = {} }: { draft?: Record<string, unknown> }) {
  const [answers, setAnswers] = useState(draft);
  return (
    <>
      <CvImportStep
        answers={answers}
        setAnswer={(key, value) => setAnswers((a) => ({ ...a, [key]: value }))}
      />
      <output data-testid="answers">{JSON.stringify(answers)}</output>
    </>
  );
}

function renderStep() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Harness />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.profileImportStart).mockReset();
  vi.mocked(commands.profileImportProposals).mockReset();
  vi.mocked(commands.profileImportConfirm).mockReset();
  vi.mocked(commands.profileImportDiscard).mockReset();
  vi.mocked(open).mockReset();
  vi.mocked(commands.profileImportProposals).mockImplementation(() => ok([]));
  vi.mocked(commands.profileImportStart).mockImplementation(() => ok(outcome([])));
  vi.mocked(commands.profileImportConfirm).mockImplementation(() => ok(snapshot));
  vi.mocked(commands.profileImportDiscard).mockImplementation(() => ok(null));
});

describe("CV import step (FR-5, AC1)", () => {
  it("picks files and imports them with the wizard's language", async () => {
    vi.mocked(open).mockResolvedValue(["/home/ada/cv.pdf", "/home/ada/cv-es.docx"]);
    vi.mocked(commands.profileImportStart).mockImplementation(() => ok(outcome([proposal()])));
    renderStep();

    fireEvent.click(screen.getByRole("button", { name: "Choose files" }));
    await screen.findByText("/home/ada/cv.pdf");
    fireEvent.click(screen.getByRole("button", { name: "Import my CV" }));

    await waitFor(() =>
      expect(commands.profileImportStart).toHaveBeenCalledWith(
        ["/home/ada/cv.pdf", "/home/ada/cv-es.docx"],
        null,
        expect.any(String),
      ),
    );
    expect(await screen.findByText("Backend engineer at Nimbus since 2020")).toBeTruthy();
  });

  it("shows every proposal with its source excerpt, verbatim and untruncated", async () => {
    const long =
      "Led the migration of the billing pipeline to a distributed architecture across three teams, cutting p99 latency by half and removing sixteen hours of nightly batch work.";
    vi.mocked(commands.profileImportProposals).mockImplementation(() =>
      ok([proposal({ text_en: long, source_excerpt: long })]),
    );
    renderStep();

    const [text] = await screen.findAllByText(long);
    expect(text.tagName).toBe("P");
    expect(text.className).toContain("whitespace-pre-wrap");
    expect(text.className).not.toContain("truncate");
    expect(text.className).not.toContain("line-clamp");
    expect(screen.getByText(long, { selector: "blockquote" })).toBeTruthy();
    expect(screen.getByText("From your CV:")).toBeTruthy();
    // AD-7: the excerpt is text, not markup.
    expect(text.textContent).toBe(long);
  });

  it("confirms a proposal exactly as proposed", async () => {
    vi.mocked(commands.profileImportProposals).mockImplementation(() => ok([proposal()]));
    renderStep();

    fireEvent.click(await screen.findByRole("button", { name: "Confirm" }));
    await waitFor(() => expect(commands.profileImportConfirm).toHaveBeenCalledWith("prop_1", null));
  });

  it("edits a proposal and confirms the edited text", async () => {
    vi.mocked(commands.profileImportProposals).mockImplementation(() =>
      ok([proposal({ text_es: "Ingeniero de backend en Nimbus" })]),
    );
    renderStep();

    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText("English"), {
      target: { value: "Backend engineer at Nimbus (2020–now)" },
    });
    fireEvent.change(screen.getByLabelText("Spanish"), {
      target: { value: "Ingeniero de backend en Nimbus (2020–hoy)" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save and confirm" }));

    await waitFor(() =>
      expect(commands.profileImportConfirm).toHaveBeenCalledWith("prop_1", {
        text_en: "Backend engineer at Nimbus (2020–now)",
        text_es: "Ingeniero de backend en Nimbus (2020–hoy)",
        evidence: null,
        detail_json: "{}",
      }),
    );
  });

  it("deletes a proposal the user does not want", async () => {
    vi.mocked(commands.profileImportProposals).mockImplementation(() => ok([proposal()]));
    renderStep();

    fireEvent.click(await screen.findByRole("button", { name: "Delete" }));
    await waitFor(() => expect(commands.profileImportDiscard).toHaveBeenCalledWith("prop_1"));
  });

  it("confirms every waiting proposal at once", async () => {
    vi.mocked(commands.profileImportProposals).mockImplementation(() =>
      ok([proposal(), proposal({ id: "prop_2", kind: "skill", text_en: "Rust" })]),
    );
    renderStep();

    fireEvent.click(await screen.findByRole("button", { name: "Confirm all (2)" }));
    await waitFor(() => {
      expect(commands.profileImportConfirm).toHaveBeenCalledWith("prop_1", null);
      expect(commands.profileImportConfirm).toHaveBeenCalledWith("prop_2", null);
    });
  });

  it("shows the paste box when text is pasted and imports it", async () => {
    renderStep();

    fireEvent.change(screen.getByLabelText("Or paste your CV text"), {
      target: { value: "Ada Lovelace — backend engineer, six years." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Import my CV" }));

    await waitFor(() =>
      expect(commands.profileImportStart).toHaveBeenCalledWith(
        [],
        "Ada Lovelace — backend engineer, six years.",
        expect.any(String),
      ),
    );
  });

  it("says the text could not be read for a scanned PDF (AC2)", async () => {
    vi.mocked(commands.profileImportStart).mockImplementation(() =>
      Promise.resolve({
        status: "error" as const,
        error: {
          code: "invalid_input",
          message_key: "errors.profile_import.no_text",
          details: null,
        },
      }),
    );
    vi.mocked(open).mockResolvedValue(["/home/ada/scan.pdf"]);
    renderStep();

    fireEvent.click(screen.getByRole("button", { name: "Choose files" }));
    await screen.findByText("/home/ada/scan.pdf");
    fireEvent.click(screen.getByRole("button", { name: "Import my CV" }));

    expect(await screen.findByText(/Couldn't read text from that file/)).toBeTruthy();
  });

  it("reports how many facts were found and how many were left out", async () => {
    vi.mocked(open).mockResolvedValue(["/home/ada/cv.pdf"]);
    vi.mocked(commands.profileImportStart).mockImplementation(() =>
      ok(outcome([proposal()], { accepted: 3, dropped: 1 })),
    );
    renderStep();

    fireEvent.click(screen.getByRole("button", { name: "Choose files" }));
    await screen.findByText("/home/ada/cv.pdf");
    fireEvent.click(screen.getByRole("button", { name: "Import my CV" }));

    expect(await screen.findByText(/3 fact\(s\) found; 1 couldn't be traced/)).toBeTruthy();
  });
});
