import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { type AppError, type ConnectionView, commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import enEmailErrors from "../../../../../locales/en/errors/email.json";
import esEmailErrors from "../../../../../locales/es/errors/email.json";
import EmailStep from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    emailAccounts: vi.fn(),
    emailConnectOutlook: vi.fn(),
    emailConnectGmail: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const failed = (error: AppError) => Promise.resolve({ status: "error" as const, error });

const account = (over: Partial<ConnectionView> = {}): ConnectionView => ({
  id: "acc-1",
  provider: "microsoft_graph",
  address: "diego@example.com",
  mode: "send_only",
  client_id_kind: "bundled",
  scopes: ["Mail.Send", "offline_access"],
  connected_at: "2026-10-17T09:00:00Z",
  ...over,
});

function renderStep() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <EmailStep answers={{}} setAnswer={() => undefined} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.emailAccounts).mockReset().mockReturnValue(ok([]));
  vi.mocked(commands.emailConnectOutlook).mockReset();
  vi.mocked(commands.emailConnectGmail).mockReset();
});

describe("Email step — optional connection (AC2, FR-9)", () => {
  it("says the step is optional and where to connect later", async () => {
    renderStep();
    expect(await screen.findByText(/optional/i)).toBeTruthy();
    expect(screen.getByText(/later in Connections/)).toBeTruthy();
  });

  it("lists the mailboxes already connected", async () => {
    vi.mocked(commands.emailAccounts).mockReturnValue(ok([account()]));
    renderStep();
    expect(await screen.findByText("diego@example.com")).toBeTruthy();
  });

  it("connects Outlook with the address and send-only by default", async () => {
    vi.mocked(commands.emailConnectOutlook).mockReturnValue(ok(account()));
    renderStep();

    fireEvent.change(await screen.findByLabelText("Mailbox address"), {
      target: { value: "diego@example.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect Outlook" }));

    await waitFor(() =>
      expect(commands.emailConnectOutlook).toHaveBeenCalledWith({
        address: "diego@example.com",
        mode: "send_only",
        client_id: null,
      }),
    );
    // The step keeps working after a connection: it lists the account.
    expect(await screen.findByText("diego@example.com")).toBeTruthy();
  });

  it("connects Gmail, which asks only for sending", async () => {
    vi.mocked(commands.emailConnectGmail).mockReturnValue(
      ok(account({ provider: "gmail", scopes: ["https://www.googleapis.com/auth/gmail.send"] })),
    );
    renderStep();

    fireEvent.change(await screen.findByLabelText("Mailbox address"), {
      target: { value: "diego@gmail.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect Gmail" }));

    await waitFor(() =>
      expect(commands.emailConnectGmail).toHaveBeenCalledWith({
        address: "diego@gmail.com",
        client_id: null,
        client_secret: null,
      }),
    );
  });

  it("shows why a connection failed and stays usable", async () => {
    vi.mocked(commands.emailConnectOutlook).mockReturnValue(
      failed({
        code: "unauthorized",
        message_key: "errors.email.not_authorised",
        details: {},
      }),
    );
    renderStep();

    fireEvent.change(await screen.findByLabelText("Mailbox address"), {
      target: { value: "diego@example.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect Outlook" }));

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Connect Outlook" })).toBeTruthy();
  });
});

describe("Email step — never a dead end (FX5)", () => {
  it("explains why the one-click buttons are disabled while the address is empty", async () => {
    renderStep();

    const hint = await screen.findByText(/type the mailbox address first/i);
    for (const name of ["Connect Outlook", "Connect Gmail"]) {
      const button = screen.getByRole("button", { name });
      expect(button).toBeDisabled();
      expect(button).toHaveAttribute("aria-describedby", hint.id);
    }

    fireEvent.change(screen.getByLabelText("Mailbox address"), {
      target: { value: "diego@example.com" },
    });
    expect(screen.queryByText(/type the mailbox address first/i)).toBeNull();
    expect(screen.getByRole("button", { name: "Connect Outlook" })).toBeEnabled();
  });

  it("turns 'no bundled client' into a next action with a link to Connections", async () => {
    vi.mocked(commands.emailConnectOutlook).mockReturnValue(
      failed({
        code: "invalid_input",
        message_key: "errors.email.no_bundled_client",
        details: {},
      }),
    );
    renderStep();

    fireEvent.change(await screen.findByLabelText("Mailbox address"), {
      target: { value: "diego@example.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect Outlook" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      /no one-click sign-in\. Skip for now, or add your own client under Connections > Email/i,
    );
    const link = screen.getByRole("link", { name: /open connections/i });
    expect(link).toHaveAttribute("href", "/connections");
    // The step never dead-ends: the buttons stay available for another try.
    expect(screen.getByRole("button", { name: "Connect Outlook" })).toBeEnabled();
  });

  it("says what a pending sign-in is waiting for", async () => {
    vi.mocked(commands.emailConnectOutlook).mockReturnValue(new Promise<never>(() => undefined));
    renderStep();

    fireEvent.change(await screen.findByLabelText("Mailbox address"), {
      target: { value: "diego@example.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect Outlook" }));

    expect(await screen.findByText(/finish signing in in your browser/i)).toBeTruthy();
  });

  it("ships error copy that names a place that exists, in en and es", () => {
    for (const [lang, catalog] of [
      ["en", enEmailErrors],
      ["es", esEmailErrors],
    ] as const) {
      for (const [key, copy] of Object.entries(catalog)) {
        expect(copy, `${lang} ${key}`).not.toMatch(/\b(Advanced|Avanzado)\b/);
      }
      const noBundled = catalog["errors.email.no_bundled_client"];
      expect(noBundled).toMatch(lang === "en" ? /Connections > Email/ : /Conexiones > Correo/);
    }
  });
});
