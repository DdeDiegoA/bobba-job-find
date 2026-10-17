import { createMemoryRouter, RouterProvider } from "react-router";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { save } from "@tauri-apps/plugin-dialog";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppProviders } from "@/app/providers";
import { routes } from "@/app/routes";
import type { OfferCardView, Preferences, SourceTileView } from "@/lib/ipc/bindings";
import "@/lib/i18n";

/**
 * The whole first run, end to end (S3.8 AC5): first launch lands in the wizard,
 * every step is completed with a fake CLI and fixture Sources, a recovery
 * backup is created, and finishing starts the search run and opens Today on the
 * ranked list. The core is faked at the IPC boundary the same way the step
 * tests fake it; the components, the router and the wizard are the real ones.
 */
const ipc = vi.hoisted(() => {
  const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
  return {
    ok,
    commands: {
      agentEngineSnapshot: vi.fn(),
      agentModels: vi.fn(),
      agentTestCall: vi.fn(),
      agentEngineChoose: vi.fn(),
      profileImportProposals: vi.fn(),
      profileGapsQuestions: vi.fn(),
      profileSnapshot: vi.fn(),
      preferencesSnapshot: vi.fn(),
      preferencesSave: vi.fn(),
      sourcesCatalogue: vi.fn(),
      sourcesAddRss: vi.fn(),
      emailAccounts: vi.fn(),
      emailConnectOutlook: vi.fn(),
      emailConnectGmail: vi.fn(),
      backupStatus: vi.fn(),
      backupCreate: vi.fn(),
      jobsRunNow: vi.fn(),
      applyRulesSnapshot: vi.fn(),
      onboardingStatus: vi.fn(),
      onboardingComplete: vi.fn(),
      offersInbox: vi.fn(),
      runReportLatest: vi.fn(),
      applyOutboxView: vi.fn(),
    },
    events: {
      runReportProgressed: { listen: vi.fn(() => Promise.resolve(() => undefined)) },
    },
  };
});

vi.mock("@/lib/ipc/bindings", () => ({
  commands: ipc.commands,
  events: ipc.events,
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn(), open: vi.fn() }));

const { ok, commands } = ipc;

const preferences = (): Preferences => ({
  roles: ["Backend Engineer"],
  seniority_min: null,
  seniority_max: null,
  salary_floor: null,
  locations: [],
  timezone_window: { from: 8, to: 18 },
  work_modes: { remote: true, hybrid: false, onsite: false },
  working_languages: ["en"],
  deal_breaker_keywords: [],
  deal_breaker_notes: "",
  excluded_companies: [],
});

const tile = (over: Partial<SourceTileView> = {}): SourceTileView => ({
  id: "remotive",
  name: "Remotive",
  source_type: "feed",
  markets: ["Global remote"],
  risk_tier: "safe",
  custom: false,
  enabled: true,
  status: "ok",
  last_run: null,
  offers_found: 0,
  last_error: null,
  consecutive_failures: 0,
  attribution: null,
  ...over,
});

/** The fixture Sources the run "searches": two Safe, one not for automation. */
const catalogue = (): SourceTileView[] => [
  tile(),
  tile({ id: "himalayas", name: "Himalayas", source_type: "api", markets: ["Europe"] }),
  tile({
    id: "linkedin",
    name: "LinkedIn",
    source_type: "session_board",
    risk_tier: "prohibited_automation",
  }),
];

/** The ranked Offer the fixture Sources produce. */
const card = (over: Partial<OfferCardView> = {}): OfferCardView => ({
  posting_id: "pst_01",
  title: "Senior Data Engineer",
  company: "Boba Labs",
  locations: ["Remote"],
  work_mode: "remote",
  lang: "en",
  amount_minor: null,
  currency: null,
  pay_period: null,
  sources: ["remotive"],
  first_seen: "2026-10-17T08:00:00Z",
  posted_at: null,
  deadline: null,
  fit: 81,
  quality: 71,
  rank: 78,
  verdict: "strong",
  confidence: "medium",
  lead: false,
  badges: [],
  exclusion: null,
  closing_soon: false,
  dismissed: false,
  overridden: false,
  ...over,
});

const next = () => screen.getByRole("button", { name: "Next" }) as HTMLButtonElement;

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  vi.mocked(save).mockResolvedValue("/home/me/boba-backup.boba");

  vi.mocked(commands.agentEngineSnapshot).mockReturnValue(
    ok({
      engines: [
        {
          cli: "claude_code",
          name: "Claude Code",
          installed: true,
          version: "fake-1.0.0",
          path: "/usr/local/bin/fake-claude",
          health: "healthy",
          install_url: "https://example.com/claude",
          unsupported_tasks: [],
        },
      ],
      chosen: null,
    }),
  );
  vi.mocked(commands.agentModels).mockReturnValue(ok([]));
  vi.mocked(commands.agentTestCall).mockReturnValue(
    ok({
      ok: true,
      cli: "claude_code",
      model: "fake-1",
      latency_ms: 12,
      reply: "pong",
      failure_reason: null,
      message_key: null,
    }),
  );
  vi.mocked(commands.agentEngineChoose).mockReturnValue(
    ok({ cli: "claude_code", model: "fake-1" }),
  );

  vi.mocked(commands.profileImportProposals).mockReturnValue(ok([]));
  // S3.6: the gap interview has nothing to ask in this fixture, so the step
  // renders its empty state and Next continues.
  vi.mocked(commands.profileGapsQuestions).mockReturnValue(ok([]));
  vi.mocked(commands.profileSnapshot).mockReturnValue(
    ok({ profile_version: 1, facts: [], pending_proposals: 0 }),
  );
  vi.mocked(commands.preferencesSnapshot).mockReturnValue(
    ok({ version: 0, values: preferences(), scores_stale: false }),
  );
  vi.mocked(commands.preferencesSave).mockReturnValue(
    ok({ version: 1, values: preferences(), scores_stale: true }),
  );
  vi.mocked(commands.sourcesCatalogue).mockReturnValue(ok(catalogue()));
  vi.mocked(commands.emailAccounts).mockReturnValue(ok([]));

  // The core refuses a run until the recovery backup exists (ADR-009), so the
  // status flips to "present" once one is written.
  vi.mocked(commands.backupStatus)
    .mockReturnValueOnce(
      ok({
        recovery_created_at: null,
        recovery_path: null,
        recovery_present: false,
        recovery_required: true,
        confirmation_token: "DELETE",
      }),
    )
    .mockReturnValue(
      ok({
        recovery_created_at: "2026-10-17T09:00:00Z",
        recovery_path: "/home/me/boba-backup.boba",
        recovery_present: true,
        recovery_required: false,
        confirmation_token: "DELETE",
      }),
    );
  vi.mocked(commands.backupCreate).mockReturnValue(
    ok({
      format: "boba-backup",
      app_version: "0.1.0",
      schema_version: 1,
      created_at: "2026-10-17T09:00:00Z",
    }),
  );

  // The core's first-launch gate, as the real one behaves: not done until
  // onboarding_complete writes the row, then done for every later read.
  const completedAt = { value: null as string | null };
  const statusNow = () =>
    ok({ completed: completedAt.value !== null, completed_at: completedAt.value });
  vi.mocked(commands.onboardingStatus).mockImplementation(statusNow);
  vi.mocked(commands.onboardingComplete).mockImplementation(() => {
    completedAt.value ??= "2026-10-17T09:00:00Z";
    return statusNow();
  });

  vi.mocked(commands.jobsRunNow).mockReturnValue(ok({ run_id: "run-1", joined: false }));
  vi.mocked(commands.applyRulesSnapshot).mockReturnValue(
    ok({ auto_enabled: false, auto_confirmed: false, global_daily_cap: 3, rules: [] }),
  );
  vi.mocked(commands.offersInbox).mockReturnValue(ok([card()]));
  vi.mocked(commands.runReportLatest).mockReturnValue(ok(null));
  vi.mocked(commands.applyOutboxView).mockReturnValue(
    ok({ settings: { hold_minutes: 60 }, items: [], rules: [] }),
  );
});

describe("first run — wizard to ranked list (AC5, SM-1)", () => {
  it("completes setup with a fake CLI and fixture Sources and opens the ranked list", async () => {
    render(
      <AppProviders>
        <RouterProvider router={createMemoryRouter(routes, { initialEntries: ["/"] })} />
      </AppProviders>,
    );

    // AC1: a first launch opens setup, not an empty Today.
    expect(await screen.findByRole("dialog", { name: "Boba setup" })).toBeTruthy();

    // 1. Language & theme: keep the defaults.
    fireEvent.click(next());

    // 2. Agent CLI: the fake CLI answers its test call, then the data notice.
    fireEvent.click(await screen.findByRole("button", { name: "Test connection" }));
    fireEvent.click(await screen.findByLabelText("I understand where my data goes"));
    await waitFor(() => expect(next().disabled).toBe(false));
    fireEvent.click(next());

    // 3. Master CV: no CV on hand.
    fireEvent.click(await screen.findByRole("button", { name: "Skip for now" }));

    // 4. Gap interview: nothing to ask with this fixture, and every question
    // is skippable anyway (FR-6), so Next continues.
    expect(
      await screen.findByText(/Boba asks only what your profile can't answer/),
    ).toBeTruthy();
    fireEvent.click(next());

    // 5. Preferences: the defaults are already valid.
    fireEvent.click(next());

    // 6. Sources: the Safe Sources are pre-selected, LinkedIn is not.
    const remotive = await screen.findByRole("checkbox", { name: /Remotive/ });
    expect((remotive as HTMLInputElement).checked).toBe(true);
    expect((screen.getByRole("checkbox", { name: /LinkedIn/ }) as HTMLInputElement).checked).toBe(
      false,
    );
    fireEvent.click(next());

    // 6. Email: optional, skipped.
    expect(await screen.findByText(/This step is optional/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Skip for now" }));

    // 7. Recovery backup: required before finishing (AC3/ADR-009).
    expect(await screen.findByText(/Create a recovery backup to finish setup/)).toBeTruthy();
    expect(next().disabled).toBe(true);
    expect(commands.jobsRunNow).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Choose where to save…" }));
    await waitFor(() => expect(save).toHaveBeenCalled());
    fireEvent.change(screen.getByLabelText("Passphrase"), { target: { value: "open sesame" } });
    fireEvent.change(screen.getByLabelText("Passphrase again"), {
      target: { value: "open sesame" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create backup" }));
    expect(await screen.findByText(/Recovery backup created/)).toBeTruthy();
    await waitFor(() => expect(next().disabled).toBe(false));
    fireEvent.click(next());

    // 8. The last step: Review, 08:00, launch at login off (AC4).
    expect(await screen.findByRole("heading", { name: "You're ready" })).toBeTruthy();
    expect(await screen.findByText("Review")).toBeTruthy();
    expect(screen.getByText(/08:00/)).toBeTruthy();
    expect(screen.getByText("Off")).toBeTruthy();
    fireEvent.click(next());

    // Finishing starts the run, records completion in the core and hands over
    // to Today (AC4).
    await waitFor(() => expect(commands.jobsRunNow).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(commands.onboardingComplete).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(await screen.findByText("Senior Data Engineer")).toBeTruthy();
    expect(screen.getByText("Strong")).toBeTruthy();
  });
});
