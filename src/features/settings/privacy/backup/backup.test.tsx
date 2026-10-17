import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { save } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { type BackupStatusView, commands, type ResetReportView } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SettingsPrivacyBackup from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    backupStatus: vi.fn(),
    backupCreate: vi.fn(),
    backupRestore: vi.fn(),
    resetEverything: vi.fn(),
  },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn(), open: vi.fn() }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const status = (over: Partial<BackupStatusView> = {}): BackupStatusView => ({
  recovery_created_at: null,
  recovery_path: null,
  recovery_present: false,
  recovery_required: true,
  confirmation_token: "DELETE",
  ...over,
});

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SettingsPrivacyBackup />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(commands.backupStatus).mockImplementation(() => ok(status()));
});

describe("Settings → Privacy → Backup (FR-111, FR-112)", () => {
  it("forces a recovery backup on an unsigned build and shows the one that exists (AC1/AC2)", async () => {
    renderPage();
    expect(await screen.findByText(/No recovery backup exists yet/)).toBeTruthy();
    expect(screen.getByText(/cannot finish and no search runs/)).toBeTruthy();

    vi.mocked(commands.backupStatus).mockImplementation(() =>
      ok(
        status({
          recovery_present: true,
          recovery_required: false,
          recovery_created_at: "2026-10-17T09:00:00Z",
          recovery_path: "/home/me/recovery.boba",
        }),
      ),
    );
    renderPage();
    expect(await screen.findByText(/Recovery backup created 2026-10-17T09:00:00Z/)).toBeTruthy();
    expect(screen.getByText(/\/home\/me\/recovery.boba/)).toBeTruthy();
  });

  it("creates a backup at the chosen path with a matching passphrase (AC1)", async () => {
    vi.mocked(save).mockResolvedValue("/home/me/recovery.boba");
    vi.mocked(commands.backupCreate).mockImplementation(() =>
      ok({
        format: "boba-backup-v1",
        app_version: "0.1.0",
        schema_version: 24,
        created_at: "2026-10-17T09:00:00Z",
      }),
    );
    renderPage();

    fireEvent.click(await screen.findByRole("button", { name: /Choose where to save/ }));
    await waitFor(() => expect(save).toHaveBeenCalledOnce());

    const passphrase = screen.getByLabelText("Passphrase");
    fireEvent.change(passphrase, { target: { value: "correct horse battery" } });
    fireEvent.change(screen.getByLabelText("Passphrase again"), {
      target: { value: "correct horse battery" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create backup" }));

    await waitFor(() =>
      expect(commands.backupCreate).toHaveBeenCalledWith(
        "/home/me/recovery.boba",
        "correct horse battery",
      ),
    );
    expect(await screen.findByText(/Backup written to \/home\/me\/recovery.boba/)).toBeTruthy();
  });

  it("refuses mismatched passphrases without calling the core (AC1)", async () => {
    vi.mocked(save).mockResolvedValue("/home/me/recovery.boba");
    renderPage();

    fireEvent.click(await screen.findByRole("button", { name: /Choose where to save/ }));
    // The chosen path arrives through an async dialog promise.
    expect(await screen.findByText("/home/me/recovery.boba")).toBeTruthy();

    fireEvent.change(screen.getByLabelText("Passphrase"), { target: { value: "one" } });
    fireEvent.change(screen.getByLabelText("Passphrase again"), { target: { value: "two" } });
    fireEvent.click(screen.getByRole("button", { name: "Create backup" }));

    expect(await screen.findByText("The two passphrases do not match.")).toBeTruthy();
    expect(commands.backupCreate).not.toHaveBeenCalled();
  });

  it("restores only after an explicit confirmation and shows the pre-restore snapshot (AC3)", async () => {
    vi.mocked(commands.backupRestore).mockImplementation(() =>
      ok({
        format: "boba-backup-v1",
        app_version: "0.1.0",
        schema_version: 20,
        created_at: "2026-10-01T08:00:00Z",
        snapshot: "/home/me/.local/share/boba/snapshots/restore-20-1.db",
        schema_to: 24,
      }),
    );
    renderPage();

    fireEvent.change(await screen.findByLabelText("Backup file"), {
      target: { value: "/home/me/recovery.boba" },
    });
    fireEvent.change(screen.getByLabelText("Backup passphrase"), {
      target: { value: "correct horse battery" },
    });

    const button = screen.getByRole("button", { name: "Restore this backup" });
    expect((button as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByLabelText(/replaces everything/));
    expect((button as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(button);

    await waitFor(() =>
      expect(commands.backupRestore).toHaveBeenCalledWith(
        "/home/me/recovery.boba",
        "correct horse battery",
      ),
    );
    expect(
      await screen.findByText(
        /previous data was saved as \/home\/me\/.local\/share\/boba\/snapshots/,
      ),
    ).toBeTruthy();
  });

  it("resets only when the confirmation token is typed, then offers a restart (AC5)", async () => {
    const report: ResetReportView = {
      db_removed: true,
      removed: ["/data/browser-profile", "/data/snapshots"],
      keychain_removed: ["db-key", "audit-head", "secret:mail:01ACC"],
      remaining: [],
      data_dir: "/data",
    };
    vi.mocked(commands.resetEverything).mockImplementation(() => ok(report));
    renderPage();

    const button = await screen.findByRole("button", { name: "Delete everything" });
    expect((button as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Type DELETE to confirm"), {
      target: { value: "DELETE" },
    });
    expect((button as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(button);

    await waitFor(() => expect(commands.resetEverything).toHaveBeenCalledWith("DELETE"));
    expect(await screen.findByText("Everything was deleted.")).toBeTruthy();
    expect(screen.getByText(/Keychain entries deleted: 3/)).toBeTruthy();
    expect(screen.getByText(/remove \/data/)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Restart Boba now" }));
    expect(relaunch).toHaveBeenCalledOnce();
  });
});
