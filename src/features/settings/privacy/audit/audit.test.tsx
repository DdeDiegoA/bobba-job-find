import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { save } from "@tauri-apps/plugin-dialog";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { type AuditEntry, type AuditLogView, commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SettingsPrivacyAudit from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: { auditLog: vi.fn(), auditLogExportCsv: vi.fn() },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

function entry(over: Partial<AuditEntry> = {}): AuditEntry {
  return {
    id: "row_1",
    kind: "send",
    at: "2026-10-17T09:00:00.000Z",
    action: "apply.submitted",
    actor: "user",
    phase: "intent",
    outcome: null,
    target_kind: "application",
    target_id: "app_1",
    application_id: "app_1",
    detail_json: '{"rule":"Email x Remotive"}',
    payload_blake3: "abc123",
    hash: "ffff",
    ...over,
  };
}

function view(over: Partial<AuditLogView> = {}): AuditLogView {
  return { entries: [], chain_intact: true, chain_rows: 0, broken_at: null, ...over };
}

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SettingsPrivacyAudit />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(commands.auditLog).mockImplementation(() => ok(view()));
});

describe("Settings → Privacy → Audit Log (FR-89, SM-7)", () => {
  it("shows sends, Auto decisions and Stage changes with the Package hash and a link (AC1)", async () => {
    vi.mocked(commands.auditLog).mockImplementation(() =>
      ok(
        view({
          chain_rows: 3,
          entries: [
            entry(),
            entry({
              id: "row_2",
              kind: "decision",
              at: "2026-10-17T08:30:00.000Z",
              action: "auto.decision",
              actor: "auto",
              phase: null,
              outcome: "needs_review",
              detail_json: '{"rule":"Email x Remotive","note":"Rank 61 < 80"}',
              payload_blake3: null,
              hash: null,
            }),
            entry({
              id: "status:evt_1",
              kind: "stage_change",
              at: "2026-10-17T08:00:00.000Z",
              action: "stage.applied",
              actor: "apply_outcome",
              phase: null,
              detail_json: '{"from":"drafted","to":"applied","reason":null}',
              payload_blake3: null,
              hash: null,
            }),
          ],
        }),
      ),
    );
    renderPage();

    expect(await screen.findByText(/Hash chain verified across 3 rows/)).toBeTruthy();
    expect(screen.getByText("Application sent")).toBeTruthy();
    // The kind badge and the event label both read "Auto decision".
    expect(screen.getAllByText("Auto decision")).toHaveLength(2);
    expect(screen.getByText("Stage → Applied")).toBeTruthy();
    expect(screen.getByText("Email x Remotive · Rank 61 < 80")).toBeTruthy();
    expect(screen.getByText("drafted → applied")).toBeTruthy();
    expect(screen.getByText("abc123")).toBeTruthy();

    const link = screen.getAllByRole("link", { name: "Open the Package" })[0];
    expect(link.getAttribute("href")).toBe("#/studio?app=app_1");
  });

  it("warns when the hash chain does not verify (AC1)", async () => {
    vi.mocked(commands.auditLog).mockImplementation(() =>
      ok(view({ chain_intact: false, chain_rows: 2, broken_at: 2, entries: [entry()] })),
    );
    renderPage();
    expect(await screen.findByText(/does not verify at row 2/)).toBeTruthy();
  });

  it("exports the CSV to the path the user picked (AC1)", async () => {
    vi.mocked(commands.auditLog).mockImplementation(() =>
      ok(view({ chain_rows: 1, entries: [entry()] })),
    );
    vi.mocked(save).mockResolvedValue("/home/me/boba-audit-log.csv");
    vi.mocked(commands.auditLogExportCsv).mockImplementation(() =>
      ok("/home/me/boba-audit-log.csv"),
    );
    renderPage();

    fireEvent.click(await screen.findByRole("button", { name: "Export CSV" }));
    await waitFor(() =>
      expect(commands.auditLogExportCsv).toHaveBeenCalledWith("/home/me/boba-audit-log.csv"),
    );
    expect(await screen.findByText(/Written to \/home\/me\/boba-audit-log.csv/)).toBeTruthy();
  });

  it("pages the log instead of rendering one wall of rows", async () => {
    const entries = Array.from({ length: 60 }, (_, index) =>
      entry({
        id: `row_${index}`,
        at: `2026-10-17T09:${String(index).padStart(2, "0")}:00.000Z`,
      }),
    );
    vi.mocked(commands.auditLog).mockImplementation(() => ok(view({ chain_rows: 60, entries })));
    renderPage();

    expect(await screen.findByText("Page 1 of 2")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(await screen.findByText("Page 2 of 2")).toBeTruthy();
  });

  it("does not call the core when the dialog is cancelled", async () => {
    vi.mocked(save).mockResolvedValue(null);
    renderPage();
    fireEvent.click(await screen.findByRole("button", { name: "Export CSV" }));
    await waitFor(() => expect(save).toHaveBeenCalledOnce());
    expect(commands.auditLogExportCsv).not.toHaveBeenCalled();
  });
});
