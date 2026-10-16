import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, UnlockStatus } from "@/lib/ipc/bindings";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import Unlock from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    unlockStatus: vi.fn(),
    unlockWithPassphrase: vi.fn(),
    unlockRetryKeychain: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const failed = (error: AppError) => Promise.resolve({ status: "error" as const, error });

const locked = (messageKey = "errors.unlock.no_keychain"): UnlockStatus => ({
  state: "locked",
  message_key: messageKey,
});

const ready: UnlockStatus = { state: "ready" };

function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <Unlock />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.unlockStatus).mockReset().mockReturnValue(ok(locked()));
  vi.mocked(commands.unlockWithPassphrase).mockReset();
  vi.mocked(commands.unlockRetryKeychain).mockReset();
});

describe("unlock screen", () => {
  it("shows why Boba is locked and offers the passphrase and the keychain retry", async () => {
    renderScreen();

    expect(await screen.findByText(/no keychain/i)).toBeDefined();
    expect(screen.getByLabelText("Passphrase")).toBeDefined();
    expect(screen.getByRole("button", { name: "Unlock" })).toBeDefined();
    expect(screen.getByRole("button", { name: "Retry reading the key" })).toBeDefined();
  });

  it("sends the passphrase to the core and shows the unlocked state on success", async () => {
    vi.mocked(commands.unlockWithPassphrase).mockReturnValue(ok(ready));
    renderScreen();

    fireEvent.change(await screen.findByLabelText("Passphrase"), {
      target: { value: "correct horse battery" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Unlock" }));

    await waitFor(() =>
      expect(commands.unlockWithPassphrase).toHaveBeenCalledWith("correct horse battery"),
    );
    expect(await screen.findByText("Boba is unlocked")).toBeDefined();
    expect(screen.getByRole("link", { name: "Continue" })).toBeDefined();
  });

  it("translates a refused passphrase and never shows the raw key", async () => {
    vi.mocked(commands.unlockWithPassphrase).mockReturnValue(
      failed({
        code: "invalid_input",
        message_key: "errors.unlock.wrong_passphrase",
      } as AppError),
    );
    renderScreen();

    fireEvent.change(await screen.findByLabelText("Passphrase"), { target: { value: "nope" } });
    fireEvent.click(screen.getByRole("button", { name: "Unlock" }));

    expect(await screen.findByText(/doesn't open your data/i)).toBeDefined();
    expect(screen.queryByText("errors.unlock.wrong_passphrase")).toBeNull();
  });

  it("retries the keychain in the foreground", async () => {
    vi.mocked(commands.unlockRetryKeychain).mockReturnValue(ok(ready));
    renderScreen();

    fireEvent.click(await screen.findByRole("button", { name: "Retry reading the key" }));

    await waitFor(() => expect(commands.unlockRetryKeychain).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("Boba is unlocked")).toBeDefined();
  });

  it("shows no passphrase form once the store is open", async () => {
    vi.mocked(commands.unlockStatus).mockReturnValue(ok(ready));
    renderScreen();

    expect(await screen.findByText("Boba is unlocked")).toBeDefined();
    expect(screen.queryByLabelText("Passphrase")).toBeNull();
  });
});
