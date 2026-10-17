import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SettingsPrivacyExport from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: { exportData: vi.fn() },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SettingsPrivacyExport />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("Settings → Privacy → Export (FR-110)", () => {
  it("warns that the export is unencrypted before it can run (AC1)", async () => {
    renderPage();

    expect(await screen.findByText("This export is not encrypted")).toBeTruthy();
    const button = screen.getByRole("button", { name: "Export" }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
  });

  it("exports into the chosen folder once the warning is acknowledged (AC1)", async () => {
    vi.mocked(open).mockResolvedValue("/home/me/boba-export");
    vi.mocked(commands.exportData).mockImplementation(() =>
      ok({ destination: "/home/me/boba-export", files: 12 }),
    );
    renderPage();

    fireEvent.click(await screen.findByRole("button", { name: /Choose a folder/ }));
    expect(await screen.findByText("/home/me/boba-export")).toBeTruthy();

    const button = screen.getByRole("button", { name: "Export" }) as HTMLButtonElement;
    expect(button.disabled, "the warning comes first").toBe(true);

    fireEvent.click(screen.getByLabelText(/I understand this export is not encrypted/));
    expect(button.disabled).toBe(false);
    fireEvent.click(button);

    await waitFor(() => expect(commands.exportData).toHaveBeenCalledWith("/home/me/boba-export"));
    expect(await screen.findByText(/Wrote 12 files to \/home\/me\/boba-export/)).toBeTruthy();
  });
});
