import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  type AppError,
  type ConnectionView,
  commands,
  type EmailApplyView,
  type ManualView,
  type SendReceipt,
} from "@/lib/ipc/bindings";
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
    applyEmailReview: vi.fn(),
    applyEmailSend: vi.fn(),
    emailAccounts: vi.fn(),
    // S9.4: the bar hosts the ATS assist and the answer approve panel.
    applyAtsOptedIn: vi.fn(() => Promise.resolve({ status: "ok", data: false })),
    applyAtsOptIn: vi.fn(),
    applyAtsStart: vi.fn(),
    applyAtsConfirm: vi.fn(),
    applyAnswers: vi.fn(() =>
      Promise.resolve({ status: "ok", data: { application_id: "app_1", answers: [] } }),
    ),
    applyAnswerApprove: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const failed = (error: AppError) => Promise.resolve({ status: "error" as const, error });

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

const account = (over: Partial<ConnectionView> = {}): ConnectionView => ({
  id: "eml_1",
  provider: "microsoft_graph",
  address: "me@example.com",
  mode: "send_only",
  client_id_kind: "bundled",
  scopes: ["Mail.Send"],
  connected_at: "2026-10-16T00:00:00Z",
  ...over,
});

function emailView(overrides: Partial<EmailApplyView> = {}): EmailApplyView {
  return {
    application_id: "app_1",
    recipient: "jobs@nimbus.example",
    subject: "Application: Backend Engineer",
    body_text: "Dear Nimbus team,",
    attachments: [{ file_name: "cv-v2.pdf", kind: "cv", version: 2, bytes: 2048 }],
    answers: [],
    pending: [],
    ready: true,
    last_attempt: null,
    ...overrides,
  };
}

const receipt: SendReceipt = {
  attempt_id: "att_01",
  message_id: "<att_01@boba.local>",
  status: "applied",
};

/** The Send button, once the mailbox list has loaded and it is clickable. */
async function sendButton(name = "Send application"): Promise<HTMLButtonElement> {
  const button = (await screen.findByRole("button", { name })) as HTMLButtonElement;
  await waitFor(() => expect(button.disabled).toBe(false));
  return button;
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
  vi.mocked(commands.emailAccounts).mockReturnValue(ok([account()]) as never);
  vi.mocked(commands.applyEmailReview).mockReturnValue(ok(emailView()) as never);
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

  it("sends the reviewed email from the chosen mailbox", async () => {
    vi.mocked(commands.applyView).mockReturnValue(ok(view()) as never);
    vi.mocked(commands.applyEmailSend).mockReturnValue(ok(receipt) as never);
    renderBar();

    expect(await screen.findByText(/cv-v2\.pdf/)).toBeTruthy();
    expect(document.querySelector("[data-email-recipient]")?.textContent).toContain(
      "jobs@nimbus.example",
    );

    fireEvent.click(await sendButton());

    await waitFor(() =>
      expect(commands.applyEmailSend).toHaveBeenCalledWith("app_1", {
        account_id: "eml_1",
        subject: null,
        body_text: null,
        confirm_resend: false,
      }),
    );
    expect(await screen.findByText(/Sent\./)).toBeTruthy();
  });

  it("sends the user's edits when they change the subject or body", async () => {
    vi.mocked(commands.applyView).mockReturnValue(ok(view()) as never);
    vi.mocked(commands.applyEmailSend).mockReturnValue(ok(receipt) as never);
    renderBar();

    const subject = (await screen.findByLabelText("Subject")) as HTMLInputElement;
    fireEvent.change(subject, { target: { value: "Hello Nimbus" } });
    fireEvent.click(await sendButton());

    await waitFor(() =>
      expect(commands.applyEmailSend).toHaveBeenCalledWith(
        "app_1",
        expect.objectContaining({ subject: "Hello Nimbus" }),
      ),
    );
  });

  it("asks for the user's word before a resend after an unanswered attempt", async () => {
    const confirm: AppError = {
      code: "conflict",
      message_key: "errors.apply.confirm_resend",
    };
    vi.mocked(commands.applyView).mockReturnValue(ok(view()) as never);
    vi.mocked(commands.applyEmailSend)
      .mockReturnValueOnce(failed(confirm) as never)
      .mockReturnValue(ok(receipt) as never);
    renderBar();

    fireEvent.click(await sendButton());

    const again = await sendButton("Send anyway");
    expect(screen.getByText(/may have gone through/)).toBeTruthy();

    fireEvent.click(again);

    await waitFor(() =>
      expect(commands.applyEmailSend).toHaveBeenLastCalledWith(
        "app_1",
        expect.objectContaining({ confirm_resend: true }),
      ),
    );
  });

  it("blocks Send while a document in the package is not verified", async () => {
    vi.mocked(commands.applyView).mockReturnValue(ok(view()) as never);
    vi.mocked(commands.applyEmailReview).mockReturnValue(
      ok(emailView({ ready: false, attachments: [], pending: ["cv"] })) as never,
    );
    renderBar();

    const send = (await screen.findByRole("button", {
      name: "Send application",
    })) as HTMLButtonElement;
    expect(send.disabled).toBe(true);
    expect(screen.getByText(/Fix the documents below/)).toBeTruthy();
  });
});
