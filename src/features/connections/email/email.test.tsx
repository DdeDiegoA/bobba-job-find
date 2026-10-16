import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, ConnectionView } from "@/lib/ipc/bindings";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import ConnectionsEmail from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    emailAccounts: vi.fn(),
    emailConnectOutlook: vi.fn(),
    emailSetMode: vi.fn(),
    emailDisconnect: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const failed = (error: AppError) => Promise.resolve({ status: "error" as const, error });

const account = (over: Partial<ConnectionView> = {}): ConnectionView => ({
  id: "01ACC",
  provider: "microsoft_graph",
  address: "me@example.com",
  mode: "send_only",
  client_id_kind: "bundled",
  scopes: ["Mail.Send"],
  connected_at: "2026-10-16T00:00:00Z",
  ...over,
});

function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ConnectionsEmail />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.emailAccounts).mockReset().mockReturnValue(ok([]));
  vi.mocked(commands.emailConnectOutlook).mockReset();
  vi.mocked(commands.emailSetMode).mockReset();
  vi.mocked(commands.emailDisconnect).mockReset();
});

describe("connecting Outlook (AC1)", () => {
  it("lists the scopes it will ask for in plain language, per mode", async () => {
    renderScreen();

    // Send-only is the default: no reading scope is mentioned.
    expect(await screen.findByText(/Send mail as you/)).toBeTruthy();
    expect(screen.queryByText(/Read your mail/)).toBeNull();
    // The refresh-token scope is named too, in plain language (AC1).
    expect(screen.getByText(/Stay connected while you are away/)).toBeTruthy();

    fireEvent.click(screen.getByLabelText(/Read and send/, { selector: "input" }));

    // AC1: both delegated scopes, in plain language, before consent.
    expect(await screen.findByText(/Read your mail/)).toBeTruthy();
    expect(screen.getByText(/Send mail as you/)).toBeTruthy();
    // NFR-5: it says what it will not ask for.
    expect(screen.getByText(/No admin consent/)).toBeTruthy();
  });

  it("connects with the address, the chosen mode and an optional own client ID", async () => {
    vi.mocked(commands.emailConnectOutlook).mockReturnValue(
      ok(account({ mode: "read_send", scopes: ["Mail.Read", "Mail.Send"] })),
    );
    renderScreen();

    fireEvent.change(await screen.findByLabelText(/Mailbox address/), {
      target: { value: " me@example.com " },
    });
    fireEvent.click(screen.getByLabelText(/Read and send/, { selector: "input" }));
    fireEvent.click(screen.getByRole("button", { name: "Connect Outlook" }));

    await waitFor(() =>
      expect(commands.emailConnectOutlook).toHaveBeenCalledWith({
        address: "me@example.com",
        mode: "read_send",
        client_id: null,
      }),
    );

    // The returned connection lands in the list with its mode and scopes
    // (the form lists them too, so both places show them).
    expect(await screen.findByText("me@example.com")).toBeTruthy();
    expect(screen.getAllByText(/Read your mail/).length).toBeGreaterThan(0);
  });

  it("sends the user's own client ID when they provide one", async () => {
    vi.mocked(commands.emailConnectOutlook).mockReturnValue(ok(account()));
    renderScreen();

    fireEvent.change(await screen.findByLabelText(/Mailbox address/), {
      target: { value: "me@example.com" },
    });
    fireEvent.change(screen.getByLabelText(/your own client ID/), {
      target: { value: " my-client-id " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect Outlook" }));

    await waitFor(() =>
      expect(commands.emailConnectOutlook).toHaveBeenCalledWith({
        address: "me@example.com",
        mode: "send_only",
        client_id: "my-client-id",
      }),
    );
  });
});

describe("a failed sign-in (AC2)", () => {
  it("shows a translated reason and a way forward, never the raw key", async () => {
    vi.mocked(commands.emailConnectOutlook).mockReturnValue(
      failed({ code: "invalid_input", message_key: "errors.email.state" }),
    );
    renderScreen();

    fireEvent.change(await screen.findByLabelText(/Mailbox address/), {
      target: { value: "me@example.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect Outlook" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("did not belong to this attempt");
    expect(alert.textContent).not.toContain("errors.email");
    // AC2: no deep-link fallback — the message points at what to do instead.
    expect(alert.textContent).toContain("127.0.0.1");
  });
});

describe("a connected mailbox (AC3/AC4)", () => {
  it("switches between send-only and read and send", async () => {
    vi.mocked(commands.emailAccounts).mockReturnValue(ok([account()]));
    vi.mocked(commands.emailSetMode).mockReturnValue(
      ok(account({ mode: "read_send", scopes: ["Mail.Read", "Mail.Send"] })),
    );
    renderScreen();

    const current = await screen.findByText("me@example.com");
    const card = current.closest("li");
    expect(card).not.toBeNull();

    fireEvent.click(
      within(card as HTMLElement).getByLabelText(/Read and send/, { selector: "input" }),
    );

    await waitFor(() => expect(commands.emailSetMode).toHaveBeenCalledWith("01ACC", "read_send"));
  });

  it("refuses to fake reading when the grant does not cover it", async () => {
    vi.mocked(commands.emailAccounts).mockReturnValue(ok([account()]));
    vi.mocked(commands.emailSetMode).mockReturnValue(
      failed({ code: "conflict", message_key: "errors.email.needs_reconsent" }),
    );
    renderScreen();

    const current = await screen.findByText("me@example.com");
    const card = current.closest("li") as HTMLElement;
    fireEvent.click(within(card).getByLabelText(/Read and send/, { selector: "input" }));

    const alert = await within(card).findByRole("alert");
    expect(alert.textContent).toContain("Connect again");
  });

  it("asks before disconnecting, then deletes the connection (AC4)", async () => {
    vi.mocked(commands.emailAccounts).mockReturnValue(ok([account()]));
    vi.mocked(commands.emailDisconnect).mockReturnValue(ok(true));
    renderScreen();

    await screen.findByText("me@example.com");
    fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));

    // The confirmation names the mailbox and what happens to the tokens.
    expect(screen.getByText(/Boba revokes the access where it can/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));

    await waitFor(() => expect(commands.emailDisconnect).toHaveBeenCalledWith("01ACC"));
    await waitFor(() => expect(screen.queryByText("me@example.com")).toBeNull());
  });
});
