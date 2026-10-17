import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { save } from "@tauri-apps/plugin-dialog";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { type BackupStatusView, commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import BackupStep, { backupStepIsValid } from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: { backupStatus: vi.fn(), backupCreate: vi.fn() },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const status = (over: Partial<BackupStatusView> = {}): BackupStatusView => ({
  recovery_created_at: null,
  recovery_path: null,
  recovery_present: false,
  recovery_required: true,
  confirmation_token: "DELETE",
  ...over,
});

function renderStep(setAnswer = vi.fn(), answers: Record<string, unknown> = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <BackupStep answers={answers} setAnswer={setAnswer} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(commands.backupStatus).mockReturnValue(ok(status()));
});

describe("Backup step — the forced recovery backup (AC3, FR-111, ADR-009)", () => {
  it("blocks Next until a recovery backup exists", async () => {
    const setAnswer = vi.fn();
    renderStep(setAnswer);
    expect(await screen.findByText(/Create a recovery backup to finish setup/)).toBeTruthy();
    expect(backupStepIsValid({})).toBe(false);
    await waitFor(() => expect(setAnswer).toHaveBeenCalledWith("recoveryPresent", false));
  });

  it("never trusts a stored 'backup exists' answer the core has not confirmed", async () => {
    // A resumed draft can carry recoveryPresent: true; until backup_status
    // answers, Next must stay blocked (the core refuses completion too).
    vi.mocked(commands.backupStatus).mockReturnValue(new Promise(() => undefined));
    const setAnswer = vi.fn();
    renderStep(setAnswer, { recoveryPresent: true });

    await waitFor(() => expect(setAnswer).toHaveBeenCalledWith("recoveryPresent", false));
  });

  it("explains in plain words why the backup is the way back in", async () => {
    renderStep();
    expect(await screen.findByText(/keychain/i)).toBeTruthy();
    expect(screen.getByText(/only copy/i)).toBeTruthy();
  });

  it("lets Next pass once the core reports a backup (AC3)", async () => {
    vi.mocked(commands.backupStatus).mockReturnValue(
      ok(
        status({
          recovery_present: true,
          recovery_required: false,
          recovery_created_at: "2026-10-17T09:00:00Z",
          recovery_path: "/home/me/boba-backup.boba",
        }),
      ),
    );
    const setAnswer = vi.fn();
    renderStep(setAnswer);
    expect(await screen.findByText(/Recovery backup created/)).toBeTruthy();
    expect(screen.getByText(/\/home\/me\/boba-backup\.boba/)).toBeTruthy();
    await waitFor(() => expect(setAnswer).toHaveBeenCalledWith("recoveryPresent", true));
    expect(backupStepIsValid({ recoveryPresent: true })).toBe(true);
  });

  it("creates a passphrase-protected backup at the chosen path (AC3)", async () => {
    vi.mocked(save).mockResolvedValue("/home/me/boba-backup.boba");
    vi.mocked(commands.backupStatus)
      .mockReturnValueOnce(ok(status()))
      .mockReturnValue(
        ok(
          status({
            recovery_present: true,
            recovery_required: false,
            recovery_path: "/home/me/boba-backup.boba",
            recovery_created_at: "2026-10-17T09:00:00Z",
          }),
        ),
      );
    vi.mocked(commands.backupCreate).mockReturnValue(
      ok({
        format: "boba-backup",
        app_version: "0.1.0",
        schema_version: 1,
        created_at: "2026-10-17T09:00:00Z",
      }),
    );
    const setAnswer = vi.fn();
    renderStep(setAnswer);

    fireEvent.click(await screen.findByRole("button", { name: "Choose where to save…" }));
    await waitFor(() => expect(save).toHaveBeenCalled());

    fireEvent.change(screen.getByLabelText("Passphrase"), { target: { value: "open sesame" } });
    fireEvent.change(screen.getByLabelText("Passphrase again"), {
      target: { value: "open sesame" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create backup" }));

    await waitFor(() =>
      expect(commands.backupCreate).toHaveBeenCalledWith(
        "/home/me/boba-backup.boba",
        "open sesame",
      ),
    );
    expect(await screen.findByText(/Recovery backup created/)).toBeTruthy();
    await waitFor(() => expect(setAnswer).toHaveBeenCalledWith("recoveryPresent", true));
  });

  it("refuses to create one with mismatched passphrases", async () => {
    vi.mocked(save).mockResolvedValue("/home/me/boba-backup.boba");
    renderStep();

    fireEvent.click(await screen.findByRole("button", { name: "Choose where to save…" }));
    await waitFor(() => expect(save).toHaveBeenCalled());
    fireEvent.change(screen.getByLabelText("Passphrase"), { target: { value: "one" } });
    fireEvent.change(screen.getByLabelText("Passphrase again"), { target: { value: "two" } });
    fireEvent.click(screen.getByRole("button", { name: "Create backup" }));

    expect(await screen.findByText(/do not match/)).toBeTruthy();
    expect(commands.backupCreate).not.toHaveBeenCalled();
  });
});
