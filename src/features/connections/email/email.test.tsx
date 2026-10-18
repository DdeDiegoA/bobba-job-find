import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import "@testing-library/jest-dom/vitest";
import { openUrl } from "@tauri-apps/plugin-opener";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, ConnectionView } from "@/lib/ipc/bindings";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import ConnectionsEmail from "./index";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn(() => Promise.resolve()) }));

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    emailAccounts: vi.fn(),
    emailConnectOutlook: vi.fn(),
    emailConnectGmail: vi.fn(),
    emailConnectImapSmtp: vi.fn(),
    emailSetMode: vi.fn(),
    emailDisconnect: vi.fn(),
    emailBundledClients: vi.fn(),
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

/** Both connect forms share label wording, so the tests scope to one of them. */
async function formOf(heading: string) {
  const title = await screen.findByRole("heading", { name: heading });
  return title.closest("form") as HTMLElement;
}

const outlookForm = () => formOf("Outlook / Microsoft 365");
const gmailForm = () => formOf("Gmail");
const imapForm = () => formOf("Other providers (IMAP and SMTP)");

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
  vi.mocked(commands.emailConnectGmail).mockReset();
  vi.mocked(commands.emailConnectImapSmtp).mockReset();
  vi.mocked(commands.emailSetMode).mockReset();
  vi.mocked(commands.emailDisconnect).mockReset();
  // Both providers bundled by default, so the existing tests keep their path.
  vi.mocked(commands.emailBundledClients)
    .mockReset()
    .mockReturnValue(ok({ outlook: true, gmail: true }));
  vi.mocked(openUrl).mockClear();
});

describe("a build with no bundled clients (FX5b)", () => {
  it("makes your own client ID the main path, with the guide one click away (AC4)", async () => {
    vi.mocked(commands.emailBundledClients).mockReturnValue(ok({ outlook: false, gmail: false }));
    renderScreen();

    const outlook = await outlookForm();
    // No "Advanced": with nothing bundled, this is how the form works.
    expect(await within(outlook).findByLabelText(/^Your own client ID/)).toBeRequired();
    expect(within(outlook).queryByText(/Advanced/)).toBeNull();

    fireEvent.click(within(outlook).getByRole("button", { name: /read the guide/i }));
    expect(openUrl).toHaveBeenCalledWith(expect.stringMatching(/email-own-client\.md$/));

    const gmail = await gmailForm();
    expect(await within(gmail).findByLabelText(/^Your own client ID/)).toBeRequired();
    expect(within(gmail).getByLabelText(/^Your client secret/)).toBeRequired();
    expect(within(gmail).queryByText(/Advanced/)).toBeNull();
  });

  it("keeps the advanced wording and optional fields when a client is bundled (AC3)", async () => {
    renderScreen();

    const outlook = await outlookForm();
    const clientId = await within(outlook).findByLabelText(/Advanced: your own client ID/);
    expect(clientId).not.toBeRequired();
    expect(within(outlook).queryByRole("button", { name: /read the guide/i })).toBeNull();
  });
});

describe("connecting Outlook (AC1)", () => {
  it("lists the scopes it will ask for in plain language, per mode", async () => {
    renderScreen();
    const form = await outlookForm();

    // Send-only is the default: no reading scope is mentioned.
    expect(within(form).getByText(/Send mail as you/)).toBeTruthy();
    expect(within(form).queryByText(/Read your mail/)).toBeNull();
    // The refresh-token scope is named too, in plain language (AC1).
    expect(within(form).getByText(/Stay connected while you are away/)).toBeTruthy();

    fireEvent.click(within(form).getByLabelText(/Read and send/, { selector: "input" }));

    // AC1: both delegated scopes, in plain language, before consent.
    expect(within(form).findByText(/Read your mail/)).toBeTruthy();
    expect(within(form).getByText(/Send mail as you/)).toBeTruthy();
    // NFR-5: it says what it will not ask for.
    expect(within(form).getByText(/No admin consent/)).toBeTruthy();
  });

  it("connects with the address, the chosen mode and an optional own client ID", async () => {
    vi.mocked(commands.emailConnectOutlook).mockReturnValue(
      ok(account({ mode: "read_send", scopes: ["Mail.Read", "Mail.Send"] })),
    );
    renderScreen();
    const form = await outlookForm();

    fireEvent.change(within(form).getByLabelText(/Mailbox address/), {
      target: { value: " me@example.com " },
    });
    fireEvent.click(within(form).getByLabelText(/Read and send/, { selector: "input" }));
    fireEvent.click(within(form).getByRole("button", { name: "Connect Outlook" }));

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
    const form = await outlookForm();

    fireEvent.change(within(form).getByLabelText(/Mailbox address/), {
      target: { value: "me@example.com" },
    });
    fireEvent.change(within(form).getByLabelText(/your own client ID/), {
      target: { value: " my-client-id " },
    });
    fireEvent.click(within(form).getByRole("button", { name: "Connect Outlook" }));

    await waitFor(() =>
      expect(commands.emailConnectOutlook).toHaveBeenCalledWith({
        address: "me@example.com",
        mode: "send_only",
        client_id: "my-client-id",
      }),
    );
  });
});

describe("connecting Gmail (AC1)", () => {
  const gmailAccount = (over: Partial<ConnectionView> = {}) =>
    account({
      provider: "gmail",
      address: "me@gmail.com",
      scopes: ["https://www.googleapis.com/auth/gmail.send"],
      ...over,
    });

  it("starts send-only and asks for gmail.send only", async () => {
    renderScreen();
    const form = await gmailForm();

    // AC1: the one scope, in plain language, before consent.
    expect(within(form).getByText(/Send mail as you/)).toBeTruthy();
    expect(within(form).queryByText(/Read your mail/)).toBeNull();
    expect(within(form).getByText(/Boba never reads your Gmail/)).toBeTruthy();
    expect(
      (within(form).getByRole("radio", { name: /Send only/ }) as HTMLInputElement).checked,
    ).toBe(true);
  });

  it("connects send-only with the address and an optional own client id and secret", async () => {
    vi.mocked(commands.emailConnectGmail).mockReturnValue(ok(gmailAccount()));
    renderScreen();
    const form = await gmailForm();

    fireEvent.change(within(form).getByLabelText(/Mailbox address/), {
      target: { value: " me@gmail.com " },
    });
    fireEvent.change(within(form).getByLabelText(/your own client ID/), {
      target: { value: " my-client-id.apps.googleusercontent.com " },
    });
    fireEvent.change(within(form).getByLabelText(/your client secret/), {
      target: { value: " GOCSPX-public " },
    });
    fireEvent.click(within(form).getByRole("button", { name: "Connect Gmail" }));

    await waitFor(() =>
      expect(commands.emailConnectGmail).toHaveBeenCalledWith({
        address: "me@gmail.com",
        mode: "send_only",
        client_id: "my-client-id.apps.googleusercontent.com",
        client_secret: "GOCSPX-public",
      }),
    );

    // The connection lands in the list as a Gmail, send-only mailbox.
    const row = (await screen.findByText("me@gmail.com")).closest("li") as HTMLElement;
    expect(within(row).getByText(/Send mail as you/)).toBeTruthy();
  });

  it("offers reading only with your own client, and asks for the read scope then", async () => {
    vi.mocked(commands.emailConnectGmail).mockReturnValue(ok(gmailAccount({ mode: "read_send" })));
    renderScreen();
    const form = await gmailForm();

    // Reading is opt-in and says what it needs before it is chosen.
    fireEvent.click(within(form).getByRole("radio", { name: /Read and send/ }));

    // Without a client id the form says what is missing and refuses to submit.
    expect(within(form).getByText(/needs your own Google client ID/)).toBeTruthy();
    expect(
      within(form).getByRole("button", { name: "Connect Gmail" }).hasAttribute("disabled"),
    ).toBe(true);
    expect(within(form).queryByText(/Read your mail/)).toBeNull();

    fireEvent.change(within(form).getByLabelText(/Mailbox address/), {
      target: { value: "me@gmail.com" },
    });
    fireEvent.change(within(form).getByLabelText(/your own client ID/), {
      target: { value: "my-client-id.apps.googleusercontent.com" },
    });
    fireEvent.change(within(form).getByLabelText(/your client secret/), {
      target: { value: "GOCSPX-public" },
    });

    // The read scope appears, in plain language, only once it can be asked for.
    expect(within(form).getByText(/Read your mail/)).toBeTruthy();
    // And what a Testing-status Google project means for the connection.
    expect(within(form).getByText(/expires the access after 7 days/)).toBeTruthy();

    fireEvent.click(within(form).getByRole("button", { name: "Connect Gmail" }));
    await waitFor(() =>
      expect(commands.emailConnectGmail).toHaveBeenCalledWith({
        address: "me@gmail.com",
        mode: "read_send",
        client_id: "my-client-id.apps.googleusercontent.com",
        client_secret: "GOCSPX-public",
      }),
    );
  });
});

describe("connecting an IMAP/SMTP account (AC2)", () => {
  const imapAccount = (over: Partial<ConnectionView> = {}) =>
    account({
      provider: "imap_smtp",
      address: "me@example.com",
      mode: "read_send",
      scopes: [],
      ...over,
    });

  function fill(form: HTMLElement, security: "TLS" | "STARTTLS" = "TLS") {
    fireEvent.change(within(form).getByLabelText(/Mailbox address/), {
      target: { value: " me@example.com " },
    });
    fireEvent.change(within(form).getByLabelText("IMAP server"), {
      target: { value: " imap.example.com " },
    });
    fireEvent.change(within(form).getByLabelText("SMTP server"), {
      target: { value: " smtp.example.com " },
    });
    fireEvent.change(within(form).getByLabelText(/App password/), {
      target: { value: "app-password" },
    });
    if (security === "STARTTLS") {
      fireEvent.change(within(form).getByLabelText("IMAP encryption"), {
        target: { value: "start_tls" },
      });
    }
  }

  it("tests before saving: a refused password is a translated error and no row", async () => {
    vi.mocked(commands.emailConnectImapSmtp).mockReturnValue(
      failed({ code: "unauthorized", message_key: "errors.email.probe_credentials" }),
    );
    renderScreen();
    const form = await imapForm();
    fill(form);
    fireEvent.click(within(form).getByRole("button", { name: "Test and save" }));

    // The endpoints and the credential go to the command that probes first.
    await waitFor(() =>
      expect(commands.emailConnectImapSmtp).toHaveBeenCalledWith({
        address: "me@example.com",
        username: "me@example.com",
        credential: "app-password",
        credential_kind: "password",
        imap_host: "imap.example.com",
        imap_port: 993,
        imap_security: "tls",
        smtp_host: "smtp.example.com",
        smtp_port: 465,
        smtp_security: "tls",
      }),
    );

    const alert = await within(form).findByRole("alert");
    expect(alert.textContent).toContain("did not accept that password");
    expect(alert.textContent).not.toContain("errors.email");
    // Nothing was saved: the mailbox is not in the list.
    expect(screen.queryByRole("heading", { name: "me@example.com" })).toBeNull();
  });

  it("offers the ports that belong to each encryption choice", async () => {
    vi.mocked(commands.emailConnectImapSmtp).mockReturnValue(ok(imapAccount()));
    renderScreen();
    const form = await imapForm();

    // STARTTLS on 143/587 is one choice away, and the defaults follow it.
    expect((within(form).getByLabelText("IMAP port") as HTMLInputElement).value).toBe("993");
    expect((within(form).getByLabelText("SMTP port") as HTMLInputElement).value).toBe("465");

    fill(form, "STARTTLS");

    expect((within(form).getByLabelText("IMAP port") as HTMLInputElement).value).toBe("143");
    fireEvent.click(within(form).getByRole("button", { name: "Test and save" }));

    await waitFor(() =>
      expect(commands.emailConnectImapSmtp).toHaveBeenCalledWith(
        expect.objectContaining({
          imap_port: 143,
          imap_security: "start_tls",
          smtp_port: 465,
          smtp_security: "tls",
        }),
      ),
    );

    // The saved row says what it does, and offers no mode switch: an IMAP/SMTP
    // account has no OAuth grant to switch.
    const row = (await screen.findByRole("heading", { name: "me@example.com" })).closest(
      "li",
    ) as HTMLElement;
    expect(within(row).getByText(/never changes anything on the server/)).toBeTruthy();
    expect(within(row).queryByLabelText(/Read and send/, { selector: "input" })).toBeNull();
  });
});

describe("a failed sign-in (AC2)", () => {
  it("shows a translated reason and a way forward, never the raw key", async () => {
    vi.mocked(commands.emailConnectOutlook).mockReturnValue(
      failed({ code: "invalid_input", message_key: "errors.email.state" }),
    );
    renderScreen();
    const form = await outlookForm();

    fireEvent.change(within(form).getByLabelText(/Mailbox address/), {
      target: { value: "me@example.com" },
    });
    fireEvent.click(within(form).getByRole("button", { name: "Connect Outlook" }));

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
