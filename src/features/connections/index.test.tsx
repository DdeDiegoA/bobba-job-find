import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, SourceTileView } from "@/lib/ipc/bindings";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import ConnectionsIndex from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    sourcesCatalogue: vi.fn(),
    sourcesTestRss: vi.fn(),
    sourcesAddRss: vi.fn(),
    sourcesRemoveRss: vi.fn(),
    emailAccounts: vi.fn(),
    emailConnectOutlook: vi.fn(),
    emailConnectGmail: vi.fn(),
    emailConnectImapSmtp: vi.fn(),
    emailSetMode: vi.fn(),
    emailDisconnect: vi.fn(),
    sessionSnapshot: vi.fn(),
    sessionOptIn: vi.fn(),
    sessionOptOut: vi.fn(),
    sessionSignOut: vi.fn(),
    sessionResume: vi.fn(),
    capabilitiesSnapshot: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const failed = (error: AppError) => Promise.resolve({ status: "error" as const, error });

const LOCKED: AppError = {
  code: "locked",
  message_key: "errors.store.locked",
};

function tile(over: Partial<SourceTileView> = {}): SourceTileView {
  return {
    id: "remotive",
    name: "Remotive",
    source_type: "feed",
    markets: ["remote"],
    risk_tier: "safe",
    custom: false,
    enabled: true,
    status: "ok",
    last_run: null,
    offers_found: 0,
    last_error: null,
    consecutive_failures: 0,
    attribution: null,
    auto_opt_in: false,
    ...over,
  };
}

const linkedin = () =>
  tile({
    id: "linkedin",
    name: "LinkedIn",
    source_type: "session_board",
    risk_tier: "prohibited_automation",
  });

const snapshot = {
  warning_version: "v1",
  cookie_protection: "macos_keychain",
  requires_extra_ack: false,
  boards: [
    {
      source_id: "linkedin",
      name: "LinkedIn",
      enabled: false,
      opted_in: false,
      accepted_at: null,
      accepted_warning_version: null,
      pause: null,
      weak_ack: false,
    },
  ],
};

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ConnectionsIndex />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.sourcesCatalogue)
    .mockReset()
    .mockReturnValue(ok([tile()]));
  vi.mocked(commands.emailAccounts).mockReset().mockReturnValue(ok([]));
  vi.mocked(commands.sessionSnapshot)
    .mockReset()
    .mockReturnValue(ok(snapshot) as never);
  vi.mocked(commands.capabilitiesSnapshot).mockReset();
});

describe("Connections container (FX3)", () => {
  it("mounts the Sources and Email leaves under exactly one h1", async () => {
    renderPage();

    expect(await screen.findByRole("heading", { name: "Remotive" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Connections", level: 1 })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Sources", level: 2 })).toBeTruthy();
    expect(screen.getByRole("heading", { name: /^Email/, level: 2 })).toBeTruthy();
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
  });

  it("opens the session opt-in for a session-board source with its id and name", async () => {
    vi.mocked(commands.sourcesCatalogue).mockReturnValue(ok([tile(), linkedin()]));
    renderPage();

    fireEvent.click(await screen.findByRole("button", { name: "Session settings for LinkedIn" }));

    const card = await waitFor(() => {
      const el = document.querySelector<HTMLElement>('[data-source="linkedin"]');
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });
    // The card carries the Source's id (prop) and renders its name (prop).
    expect(within(card).getByRole("heading", { name: "LinkedIn" })).toBeTruthy();
    expect(document.querySelectorAll('[data-source="linkedin"]')).toHaveLength(1);
  });

  it("shows a section empty state instead of a blank page", async () => {
    vi.mocked(commands.sourcesCatalogue).mockReturnValue(ok([]));
    renderPage();

    expect(await screen.findByText("No sources yet")).toBeTruthy();
    expect(await screen.findByText("No mailbox connected yet")).toBeTruthy();
    expect(await screen.findByText("No session boards yet")).toBeTruthy();
  });

  it("shows the existing locked state and does not crash", async () => {
    vi.mocked(commands.sourcesCatalogue).mockReturnValue(failed(LOCKED));
    vi.mocked(commands.emailAccounts).mockReturnValue(failed(LOCKED));
    vi.mocked(commands.sessionSnapshot).mockReturnValue(failed(LOCKED) as never);
    renderPage();

    expect(
      await screen.findByText("Boba can't reach your data yet. Unlock it to continue."),
    ).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Connections", level: 1 })).toBeTruthy();
  });

  it("renders in Limited mode, with no AI engine connected", async () => {
    vi.mocked(commands.capabilitiesSnapshot).mockReturnValue(
      ok({
        limited: true,
        agent: null,
        capabilities: [
          { id: "sources", available: true, reason: null },
          { id: "email_classification", available: false, reason: "no_agent" },
        ],
      }) as never,
    );
    renderPage();

    expect(await screen.findByRole("heading", { name: "Sources", level: 2 })).toBeTruthy();
    expect(screen.getByRole("heading", { name: /^Email/, level: 2 })).toBeTruthy();
  });
});
