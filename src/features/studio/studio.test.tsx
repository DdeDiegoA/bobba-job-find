import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { ReactNode } from "react";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type StudioDocument, type StudioView } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import StudioIndex from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    trackerBoard: vi.fn(),
    studioView: vi.fn(),
    studioEdit: vi.fn(),
    studioRender: vi.fn(),
    studioGenerate: vi.fn(),
    studioRegenerate: vi.fn(),
    studioExportDocx: vi.fn(),
    // The apply bar (S9.2) mounts at the bottom of this screen.
    applyView: vi.fn(() =>
      Promise.resolve({
        status: "ok" as const,
        data: {
          application_id: "app_1",
          posting_id: "pst_1",
          title: "Backend Engineer",
          company: "Nimbus",
          status: "drafted",
          channels: [{ kind: "manual" as const, url: "https://nimbus.example/jobs/1" }],
          documents: [],
          answers: [],
          pending: [],
          ready: true,
        },
      }),
    ),
    applyMarkApplied: vi.fn(),
    applyExportPdfs: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

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
          ],
        },
      ],
      paragraphs: [],
    },
    ready: true,
    checks: {
      fabrication_ready: true,
      layout: { pages: 1, orphans: 0, page_budget: 2, passes: true },
      text_layer: {
        passed: true,
        missing_contact: [],
        missing_keywords: [],
        out_of_order: [],
        undecodable: [],
      },
      verified: true,
      cut: [],
    },
    previous: { id: "doc_cv_1", version: 1 },
    diff: [],
    review: {
      document_id: "doc_cv_1",
      document_version: 1,
      findings: [
        {
          severity: "high",
          target: "cv",
          message: "No metric on the billing line.",
          suggestion: "Name the scale.",
        },
      ],
    },
    ...overrides,
  } as StudioDocument;
}

function view(cvDocument: StudioDocument | null): StudioView {
  return {
    application_id: "app_1",
    posting_id: "pst_1",
    page_budget: 2,
    templates: [
      { id: "cv-classic", kind: "cv", default: true },
      { id: "cover-classic", kind: "cover_letter", default: true },
    ],
    requirements: [
      { requirement: "Rust and Postgres", covered: ["rust", "postgres"], gaps: [] },
      { requirement: "Cloud infrastructure", covered: [], gaps: ["terraform"] },
    ],
    cv: cvDocument,
    cover: null,
  } as StudioView;
}

function renderStudio(path: string) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[path]}>{children}</MemoryRouter>
    </QueryClientProvider>
  );
  return render(<StudioIndex />, { wrapper });
}

beforeEach(() => {
  vi.mocked(commands.trackerBoard).mockReset();
  vi.mocked(commands.trackerBoard).mockReturnValue(ok({ columns: [] }));
  vi.mocked(commands.studioView).mockReset();
  vi.mocked(commands.studioEdit).mockReset();
  vi.mocked(commands.studioExportDocx).mockReset();
  vi.mocked(commands.studioRender).mockReset();
  vi.mocked(commands.studioGenerate).mockReset();
  vi.mocked(commands.studioRegenerate).mockReset();
});

describe("the studio screen (AC1/AC2, EXPERIENCE §9.5)", () => {
  it("reads the draft of the application the route names", async () => {
    vi.mocked(commands.studioView).mockReturnValue(ok(view(cv())));

    renderStudio("/studio?app=app_1");

    const editor = await screen.findByRole("region", { name: "CV" });
    expect(within(editor).getByText("Shipped a billing API.")).toBeTruthy();
    expect(screen.getByText("Rust and Postgres")).toBeTruthy();
    expect(screen.getByText("No metric on the billing line.")).toBeTruthy();
    expect(commands.studioView).toHaveBeenCalledWith("app_1");
  });

  it("asks for an application when the route names none, and offers the Tracker's", async () => {
    vi.mocked(commands.trackerBoard).mockReturnValue(
      ok({
        columns: [
          {
            stage: "saved",
            status: "saved",
            terminal: false,
            count: 1,
            cards: [
              {
                id: "app_1",
                status: "saved",
                stage: "saved",
                title: "Backend Engineer",
                company: "Boba Labs",
                next_action: null,
              },
            ],
          },
        ],
      }),
    );

    renderStudio("/studio");

    expect(screen.getByText("Pick an application")).toBeTruthy();
    const link = await screen.findByRole("link", { name: /Backend Engineer/ });
    expect(link.getAttribute("href")).toBe("/studio?app=app_1");
    expect(commands.studioView).not.toHaveBeenCalled();
  });

  it("reads the checks and unblocks a verified, ready draft", async () => {
    vi.mocked(commands.studioView).mockReturnValue(ok(view(cv())));

    renderStudio("/studio?app=app_1");

    expect(await screen.findByText("Verified: layout, ATS and facts all pass.")).toBeTruthy();
    expect(screen.getByText("Ready to send.")).toBeTruthy();
  });

  it("blocks a draft whose facts or checks do not pass", async () => {
    vi.mocked(commands.studioView).mockReturnValue(
      ok(
        view(
          cv({
            ready: false,
            checks: {
              fabrication_ready: false,
              layout: { pages: 1, orphans: 0, page_budget: 2, passes: true },
              text_layer: {
                passed: false,
                missing_contact: [],
                missing_keywords: ["rust"],
                out_of_order: [],
                undecodable: [],
              },
              verified: false,
              cut: [],
            },
          }),
        ),
      ),
    );

    renderStudio("/studio?app=app_1");

    expect(
      await screen.findByText("Not ready: fix the flagged lines or render again."),
    ).toBeTruthy();
    expect(screen.getByText("A line names a fact your Profile does not have.")).toBeTruthy();
    expect(screen.getByText("1 keyword(s) missing from the text layer.")).toBeTruthy();
  });

  it("exports the DOCX and names the file it wrote", async () => {
    vi.mocked(commands.studioView).mockReturnValue(ok(view(cv())));
    vi.mocked(commands.studioExportDocx).mockReturnValue(
      ok({
        path: "/tmp/exports/boba-app_1-cv-v2.docx",
        file_name: "boba-app_1-cv-v2.docx",
        version: 2,
      }),
    );

    renderStudio("/studio?app=app_1");
    fireEvent.click(await screen.findByRole("button", { name: "Export DOCX" }));

    await waitFor(() => expect(commands.studioExportDocx).toHaveBeenCalledWith("app_1", "cv"));
    expect(await screen.findByText("Saved boba-app_1-cv-v2.docx")).toBeTruthy();
  });

  it("sends an inline edit to the core as a new version request", async () => {
    vi.mocked(commands.studioView).mockReturnValue(ok(view(cv())));
    vi.mocked(commands.studioEdit).mockReturnValue(ok(view(cv())));

    renderStudio("/studio?app=app_1");
    const editor = await screen.findByRole("region", { name: "CV" });
    const row = within(editor).getByText("Shipped a billing API.").closest("li") as HTMLElement;
    fireEvent.click(row.querySelector("button:last-child") as HTMLButtonElement);
    fireEvent.change(screen.getByLabelText("The line's text"), {
      target: { value: "Shipped the billing API for 2M users." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(commands.studioEdit).toHaveBeenCalledWith({
        application_id: "app_1",
        kind: "cv",
        target: { slot: "cv_item", section: 0, item: 0 },
        text: "Shipped the billing API for 2M users.",
      }),
    );
  });

  it("shows a refused action in the user's language", async () => {
    vi.mocked(commands.studioView).mockReturnValue(ok(view(cv())));
    vi.mocked(commands.studioExportDocx).mockReturnValue(
      Promise.resolve({
        status: "error" as const,
        error: {
          code: "internal" as const,
          message_key: "errors.studio.export_write",
          details: undefined,
        },
      }),
    );

    renderStudio("/studio?app=app_1");
    fireEvent.click(await screen.findByRole("button", { name: "Export DOCX" }));

    expect(await screen.findByText("The DOCX could not be written to disk.")).toBeTruthy();
  });

  it("offers no draft yet without pretending the application was drafted", async () => {
    vi.mocked(commands.studioView).mockReturnValue(ok(view(null)));

    renderStudio("/studio?app=app_1");

    expect(await screen.findByText("No drafts yet")).toBeTruthy();
  });
});
