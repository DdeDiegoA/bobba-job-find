import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, ManualOffer } from "@/lib/ipc/bindings";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import InboxManualAdd from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    ingestManualUrl: vi.fn(),
    ingestManualText: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const failed = (error: AppError) => Promise.resolve({ status: "error" as const, error });

const offer = (over: Partial<ManualOffer> = {}): ManualOffer => ({
  id: "pst_01",
  title: "Senior Rust Engineer",
  company: "Acme",
  source_id: "manual",
  source_name: "Manual",
  locations: ["Remote"],
  work_mode: "remote",
  amount_minor: 12_000_000,
  currency: "USD",
  pay_period: "year",
  apply_url: "https://acme.example/1",
  lang: "en",
  ...over,
});

beforeEach(() => {
  vi.mocked(commands.ingestManualUrl).mockReset();
  vi.mocked(commands.ingestManualText).mockReset();
});

describe("pasting the posting text (AC2)", () => {
  it("creates an Offer and shows it with Source Manual", async () => {
    vi.mocked(commands.ingestManualText).mockReturnValue(ok(offer()));
    render(<InboxManualAdd />);

    fireEvent.click(screen.getByRole("radio", { name: "The text" }));
    fireEvent.change(screen.getByLabelText("Posting text"), {
      target: { value: "Senior Rust Engineer\nAcme" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add this job" }));

    expect(await screen.findByText("Senior Rust Engineer")).toBeTruthy();
    expect(vi.mocked(commands.ingestManualText)).toHaveBeenCalledWith("Senior Rust Engineer\nAcme");
    // AC2: the source is named on the card.
    expect(screen.getByText("Source: Manual")).toBeTruthy();
  });

  it("shows a posting that states nothing as such, without inventing a company", async () => {
    vi.mocked(commands.ingestManualText).mockReturnValue(
      ok(offer({ title: "", company: "", amount_minor: null, currency: null, pay_period: null })),
    );
    render(<InboxManualAdd />);

    fireEvent.click(screen.getByRole("radio", { name: "The text" }));
    fireEvent.change(screen.getByLabelText("Posting text"), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: "Add this job" }));

    expect(await screen.findByText("Untitled posting")).toBeTruthy();
    expect(screen.getByText("Company not stated")).toBeTruthy();
  });
});

describe("pasting a link (AC1)", () => {
  it("fetches the URL through the backend and shows the Offer", async () => {
    vi.mocked(commands.ingestManualUrl).mockReturnValue(ok(offer({ title: "Staff Engineer" })));
    render(<InboxManualAdd />);

    fireEvent.change(screen.getByLabelText("Job link"), {
      target: { value: "https://boards.example.com/acme/1" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Fetch and add" }));

    expect(await screen.findByText("Staff Engineer")).toBeTruthy();
    expect(vi.mocked(commands.ingestManualUrl)).toHaveBeenCalledWith(
      "https://boards.example.com/acme/1",
    );
  });

  it("asks for the text when the page cannot be fetched", async () => {
    vi.mocked(commands.ingestManualUrl).mockReturnValue(
      failed({
        code: "unavailable",
        message_key: "errors.manual.fetch_failed",
      }),
    );
    render(<InboxManualAdd />);

    fireEvent.change(screen.getByLabelText("Job link"), {
      target: { value: "https://linkedin.com/jobs/1" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Fetch and add" }));

    // The message says what to do, and the screen has already switched to it.
    expect(await screen.findByText(/could not fetch that page/i)).toBeTruthy();
    expect((screen.getByRole("radio", { name: "The text" }) as HTMLInputElement).checked).toBe(
      true,
    );
    expect(screen.getByLabelText("Posting text")).toBeTruthy();
  });

  it("refuses a link that is not http, with wording, never a raw key", async () => {
    vi.mocked(commands.ingestManualUrl).mockReturnValue(
      failed({ code: "invalid_input", message_key: "errors.manual.invalid_url" }),
    );
    render(<InboxManualAdd />);

    // A scheme the input accepts but Boba will not fetch: the backend refuses it
    // (the input's own `type=url` check is the browser's, not Boba's).
    fireEvent.change(screen.getByLabelText("Job link"), {
      target: { value: "file:///etc/passwd" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Fetch and add" }));

    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      "That does not look like a web link. Use an http or https address.",
    );
  });
});
