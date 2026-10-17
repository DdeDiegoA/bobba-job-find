import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type ManualView } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import StudioApplyBar from "./index";

const openUrl = vi.fn();
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: (...args: unknown[]) => openUrl(...args),
  openPath: vi.fn(() => Promise.resolve()),
}));

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    applyView: vi.fn(),
    applyMarkApplied: vi.fn(),
    applyExportPdfs: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

function view(overrides: Partial<ManualView> = {}): ManualView {
  return {
    application_id: "app_1",
    posting_id: "pst_1",
    title: "Backend Engineer",
    company: "Nimbus",
    status: "drafted",
    channels: [
      { kind: "email_apply", address: "jobs@nimbus.example" },
      { kind: "ats_form", provider: "lever", url: "https://jobs.lever.co/nimbus/abc" },
      { kind: "manual", url: "https://jobs.lever.co/nimbus/abc" },
    ],
    documents: [{ document_id: "doc_cv_2", kind: "cv", lang: "en", version: 2 }],
    answers: [
      { question: "What are your salary expectations?", answer: "€65,000", class: "salary" },
    ],
    pending: [],
    ready: true,
    ...overrides,
  };
}

function renderBar() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <StudioApplyBar applicationId="app_1" />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("StudioApplyBar", () => {
  it("shows every detected channel", async () => {
    vi.mocked(commands.applyView).mockReturnValue(ok(view()) as never);
    renderBar();

    expect(await screen.findByText("Email Apply")).toBeTruthy();
    expect(screen.getByText("ATS Form")).toBeTruthy();
    expect(screen.getByText("Manual")).toBeTruthy();
    expect(screen.getByText(/jobs@nimbus\.example/)).toBeTruthy();
  });

  it("opens the original page when the user applies by hand", async () => {
    vi.mocked(commands.applyView).mockReturnValue(ok(view()) as never);
    renderBar();

    fireEvent.click(await screen.findByRole("button", { name: "Open original page" }));

    expect(openUrl).toHaveBeenCalledWith("https://jobs.lever.co/nimbus/abc");
  });

  it("shows the verified PDFs and the approved answers", async () => {
    vi.mocked(commands.applyView).mockReturnValue(ok(view()) as never);
    renderBar();

    expect(await screen.findByText(/CV · v2/)).toBeTruthy();
    expect(screen.getByText("What are your salary expectations?")).toBeTruthy();
    expect(screen.getByText("€65,000")).toBeTruthy();
  });

  it("warns about drafts that are not verified and blocks marking applied", async () => {
    vi.mocked(commands.applyView).mockReturnValue(
      ok(view({ pending: ["cover_letter"], ready: false, documents: [] })) as never,
    );
    renderBar();

    const warning = await screen.findByRole("alert");
    expect(warning.textContent).toContain("cover letter");
    const mark = screen.getByRole("button", { name: "Mark applied" }) as HTMLButtonElement;
    expect(mark.disabled).toBe(true);
  });

  it("marks the application applied through the core", async () => {
    vi.mocked(commands.applyView).mockReturnValue(ok(view()) as never);
    vi.mocked(commands.applyMarkApplied).mockReturnValue(ok(view({ status: "applied" })) as never);
    renderBar();

    fireEvent.click(await screen.findByRole("button", { name: "Mark applied" }));

    await waitFor(() => expect(commands.applyMarkApplied).toHaveBeenCalledWith("app_1"));
    expect(await screen.findByText("Applied")).toBeTruthy();
  });

  it("saves the kit PDFs and names where they went", async () => {
    vi.mocked(commands.applyView).mockReturnValue(ok(view()) as never);
    vi.mocked(commands.applyExportPdfs).mockReturnValue(
      ok([
        { path: "/tmp/exports/apply/app_1/cv-v2.pdf", file_name: "cv-v2.pdf", version: 2 },
      ]) as never,
    );
    renderBar();

    fireEvent.click(await screen.findByRole("button", { name: "Save PDFs" }));

    expect(await screen.findByText("Saved cv-v2.pdf")).toBeTruthy();
  });
});
