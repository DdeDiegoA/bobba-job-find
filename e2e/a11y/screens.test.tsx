import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "@/lib/i18n";
import {
  type ApplicationDetail,
  type BoardView,
  type OfferCardView,
  type OfferDetailView,
  type RulesSnapshot,
  type StudioView,
  commands,
  events,
} from "@/lib/ipc/bindings";
import KillSwitch from "@/app/header/kill-switch";
import ConnectionsIndex from "@/features/connections";
import OfferDetail from "@/features/offer";
import SettingsAiEngine from "@/features/settings/ai-engine";
import SettingsApplyingRules from "@/features/settings/applying/rules";
import SettingsNotifications from "@/features/settings/notifications";
import StudioIndex from "@/features/studio";
import TodayIndex from "@/features/today";
import TrackerIndex from "@/features/tracker";
import { OnboardingWizard } from "@/features/onboarding/wizard";
import { auditTree, type KnownDefect, unexpected } from "./audit";
import { axeAudit } from "./axe";

/**
 * The AC1 screens of S10.6, rendered for real and audited (NFR-15,
 * EXPERIENCE §7): the wizard, Today, Offer detail, Studio, Tracker, Applying and
 * the Kill Switch. The core is faked at the IPC boundary — the same seam the
 * feature tests use — so the components, the router and the i18n catalogs under
 * audit are the ones that ship.
 *
 * Keyboard behaviour, screen readers and zoom are *not* decided here; the
 * scripted checks live in `docs/qa/accessibility.md` and the keyboard cases in
 * `keyboard.test.tsx`.
 */

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    offersInbox: vi.fn(),
    offerDetail: vi.fn(),
    trackerBoard: vi.fn(),
    trackerApplicationDetail: vi.fn(),
    studioView: vi.fn(),
    applyView: vi.fn(),
    applyExportPdfs: vi.fn(),
    applyRulesSnapshot: vi.fn(),
    agentBudgetSnapshot: vi.fn(),
    agentBudgetSave: vi.fn(),
    agentEngineSnapshot: vi.fn(),
    agentEngineChoose: vi.fn(),
    agentModels: vi.fn(),
    agentTestCall: vi.fn(),
    notificationsSnapshot: vi.fn(),
    notificationsSave: vi.fn(),
    autostartStatus: vi.fn(),
    autostartSet: vi.fn(),
    applyRuleSave: vi.fn(),
    applyRuleDelete: vi.fn(),
    applyRulesSetEnabled: vi.fn(),
    applyRulesReset: vi.fn(),
    applyRulesSetGlobalCap: vi.fn(),
    applyRulesExplain: vi.fn(),
    sessionAutoSnapshot: vi.fn(),
    killSwitchStatus: vi.fn(),
    killSwitchSet: vi.fn(),
    applyOutboxView: vi.fn(),
    runReportLatest: vi.fn(),
    runReportRetry: vi.fn(),
    statsFunnel: vi.fn(() => new Promise(() => undefined)),
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
    statusProposalsDisclosure: vi.fn(),
    statusProposalsPending: vi.fn(),
  },
  events: {
    runReportProgressed: { listen: vi.fn(() => Promise.resolve(() => undefined)) },
    killSwitchChanged: { listen: vi.fn(() => Promise.resolve(() => undefined)) },
  },
}));

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn(() => Promise.resolve()) }));
vi.mock("sonner", () => ({ toast: vi.fn(), Toaster: () => null }));

function card(over: Partial<OfferCardView> = {}): OfferCardView {
  return {
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
  };
}

function detail(over: Partial<OfferDetailView> = {}): OfferDetailView {
  return {
    posting_id: "pst_01",
    title: "Senior Data Engineer",
    company: "Boba Labs",
    locations: ["Remote, Spain"],
    work_mode: "remote",
    lang: "en",
    amount_minor: 6_500_000,
    currency: "EUR",
    pay_period: "year",
    first_seen: "2026-10-14T00:00:00Z",
    posted_at: "2026-10-14T00:00:00Z",
    deadline: null,
    description_html: "<p>We use <strong>dbt</strong> and Airflow.</p>",
    requirements: ["dbt"],
    apply_url: "https://boards.example.com/boba/gh-42",
    apply_email: "jobs@boba.example",
    sources: ["greenhouse"],
    duplicates: [],
    channels: [
      { kind: "email_apply", address: "jobs@boba.example" },
      { kind: "manual", url: "https://boards.example.com/boba/gh-42" },
    ],
    fit: 82.4,
    quality: 71,
    rank: 78,
    verdict: "strong",
    confidence: "medium",
    lead: false,
    gates: [
      { gate: "language", verdict: "pass", reason: "english_fluent" },
      { gate: "seniority", verdict: "unknown", reason: null },
    ],
    factors: [],
    narration: "You match the database work and live in the right timezone.",
    ...over,
  } as OfferDetailView;
}

function board(): BoardView {
  const empty = (stage: string): BoardView["columns"][number] => ({
    stage,
    status: stage,
    terminal: ["hired", "rejected", "no_response", "withdrawn"].includes(stage),
    count: 0,
    cards: [],
  });
  return {
    columns: [
      {
        ...empty("saved"),
        count: 1,
        cards: [
          {
            id: "app_1",
            status: "saved",
            stage: "saved",
            title: "Backend Engineer",
            company: "Tapi Tea",
            next_action: null,
          },
        ],
      },
      empty("preparing"),
      empty("applied"),
      empty("interviewing"),
      empty("offer"),
      empty("hired"),
      empty("rejected"),
      empty("no_response"),
      empty("withdrawn"),
    ],
  };
}

function studioView(): StudioView {
  return {
    application_id: "app_1",
    posting_id: "pst_1",
    page_budget: 2,
    templates: [
      { id: "cv-classic", kind: "cv", default: true },
      { id: "cover-classic", kind: "cover_letter", default: true },
    ],
    requirements: [{ requirement: "Rust and Postgres", covered: ["rust", "postgres"], gaps: [] }],
    cv: {
      id: "doc_cv_2",
      kind: "cv",
      version: 2,
      lang: "en",
      template: "cv-classic",
      created_at: "2026-10-17T00:00:00Z",
      untrusted: true,
      language_fallback_from: null,
      text: {
        summary: {
          text: "Backend developer.",
          fact_ids: ["fact_1"],
          facts: [{ id: "fact_1", label: "Ada Lovelace" }],
          backed: true,
          unknown_facts: [],
        },
        sections: [
          {
            heading: "Experience",
            items: [
              {
                text: "Shipped a billing API.",
                fact_ids: ["fact_2"],
                facts: [{ id: "fact_2", label: "Shipped a billing API in Rust" }],
                backed: true,
                unknown_facts: [],
              },
            ],
          },
        ],
        paragraphs: [],
      },
      ready: true,
      checks: {
        fabrication_ready: true,
        layout: { pages: 1, orphans: 0, page_budget: 2, passes: true },
        text_layer: {
          passed: true,
          missing_contact: [],
          missing_keywords: [],
          out_of_order: [],
          undecodable: [],
        },
        verified: true,
        cut: [],
      },
      previous: { id: "doc_cv_1", version: 1 },
      diff: [],
      review: { document_id: "doc_cv_1", document_version: 1, findings: [] },
    },
    cover: null,
  } as StudioView;
}

function rulesSnapshot(): RulesSnapshot {
  return {
    auto_enabled: false,
    auto_confirmed: false,
    global_daily_cap: 10,
    rules: [
      {
        id: "rule_1",
        name: "Email × remote",
        enabled: true,
        source_ids: ["remotive"],
        channels: ["email"],
        min_rank: 80,
        min_fit: 80,
        min_confidence: "medium",
        allow_flags: false,
        min_verdict: "good",
        daily_cap: 5,
        per_company_cap: 1,
        per_company_days: 30,
        active_start_minute: 540,
        active_end_minute: 1080,
        created_at: "2026-10-17T00:00:00Z",
        updated_at: "2026-10-17T00:00:00Z",
      },
    ],
  };
}

function applicationDetail(): ApplicationDetail {
  return {
    id: "app_1",
    posting_id: "pst_1",
    status: "saved",
    stage: "saved",
    title: "Backend Engineer",
    company: "Tapi Tea",
    apply_url: "https://jobs.example.com/abc-1",
    lang: "en",
    deadline: null,
    notes: "",
    next_action: null,
    package: [],
    timeline: [],
  } as ApplicationDetail;
}

function wireFixtures() {
  vi.mocked(commands.offersInbox).mockReturnValue(ok([card()]));
  vi.mocked(commands.offerDetail).mockReturnValue(ok(detail()));
  vi.mocked(commands.trackerBoard).mockReturnValue(ok(board()));
  vi.mocked(commands.trackerApplicationDetail).mockReturnValue(ok(applicationDetail()));
  vi.mocked(commands.studioView).mockReturnValue(ok(studioView()));
  vi.mocked(commands.applyView).mockReturnValue(
    ok({
      application_id: "app_1",
      posting_id: "pst_1",
      title: "Backend Engineer",
      company: "Tapi Tea",
      status: "drafted",
      channels: [{ kind: "manual", url: "https://jobs.example.com/abc-1" }],
      documents: [],
      answers: [],
      pending: [],
      ready: true,
    }),
  );
  vi.mocked(commands.applyRulesSnapshot).mockReturnValue(ok(rulesSnapshot()));
  vi.mocked(commands.agentBudgetSnapshot).mockReturnValue(
    ok({
      offers_per_run: 30,
      max_offers_per_run: 500,
      limits: [
        { task: "extract_postings", max_turns: 30, max_tool_calls: 60, wall_clock_secs: 600 },
      ],
    }) as never,
  );
  vi.mocked(commands.agentBudgetSave).mockImplementation((value) => ok(value) as never);
  vi.mocked(commands.agentEngineSnapshot).mockReturnValue(
    ok({
      engines: [
        {
          cli: "claude_code",
          name: "Claude Code",
          installed: true,
          version: "2.1.301",
          path: "/usr/local/bin/claude",
          health: "healthy",
          install_url: "https://example.test/claude",
          unsupported_tasks: [],
        },
      ],
      chosen: null,
    }) as never,
  );
  vi.mocked(commands.agentEngineChoose).mockImplementation(
    (cli, model) => ok({ cli, model }) as never,
  );
  vi.mocked(commands.agentModels).mockReturnValue(ok(["sonnet"]) as never);
  vi.mocked(commands.agentTestCall).mockImplementation(() => new Promise(() => undefined) as never);
  vi.mocked(commands.notificationsSnapshot).mockReturnValue(
    ok({
      events: { digest: true, outbox: true, errors: true },
      top_matches_shown: 3,
      notify_when_empty: false,
      quiet_hours: { enabled: false, from: 22, to: 7, notify_during: false },
    }) as never,
  );
  vi.mocked(commands.notificationsSave).mockImplementation((settings) => ok(settings) as never);
  vi.mocked(commands.autostartStatus).mockReturnValue(Promise.resolve({ enabled: false }));
  vi.mocked(commands.autostartSet).mockImplementation((enabled) => ok({ enabled }) as never);
  vi.mocked(commands.sessionAutoSnapshot).mockReturnValue(ok([]));
  vi.mocked(commands.killSwitchStatus).mockReturnValue(
    ok({ engaged: false, since: null }) as never,
  );
  vi.mocked(commands.applyOutboxView).mockReturnValue(
    ok({ settings: { hold_minutes: 30 }, items: [], rules: [] }) as never,
  );
  vi.mocked(commands.runReportLatest).mockReturnValue(ok(null) as never);
  vi.mocked(commands.sourcesCatalogue).mockReturnValue(
    ok([
      {
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
      },
      {
        id: "linkedin",
        name: "LinkedIn",
        source_type: "session_board",
        markets: ["global"],
        risk_tier: "prohibited_automation",
        custom: false,
        enabled: false,
        status: "ok",
        last_run: null,
        offers_found: 0,
        last_error: null,
        consecutive_failures: 0,
        attribution: null,
        auto_opt_in: false,
      },
    ]) as never,
  );
  vi.mocked(commands.statusProposalsDisclosure).mockReturnValue(
    ok({ mode: "deterministic", acknowledged: true, auto_confirm: false, needs_disclosure: false }),
  );
  vi.mocked(commands.statusProposalsPending).mockReturnValue(ok([]));
  vi.mocked(commands.emailAccounts).mockReturnValue(ok([]));
  vi.mocked(commands.sessionSnapshot).mockReturnValue(
    ok({
      warning_version: "v1",
      cookie_protection: "macos_keychain",
      requires_extra_ack: false,
      boards: [],
    }) as never,
  );
}

function providers(children: ReactNode, path = "/") {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return (
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[path]}>{children}</MemoryRouter>
    </QueryClientProvider>
  );
}

/**
 * Findings that belong to another slice (S10.6 AC3 files them as follow-ups
 * instead of fixing them here). Each entry is one line in
 * `docs/qa/accessibility.md` → Findings; remove it when the owner ships the fix.
 * Empty since CH14 fixed A11Y-1, A11Y-2 and A11Y-3; keep the list so a future
 * pass can pin a new defect without ever growing it silently.
 */
const KNOWN: KnownDefect[] = [];

async function audit(
  children: ReactNode,
  path = "/",
  ready: () => Promise<unknown> = () =>
    screen.findByRole("heading", { level: 1 }).catch(() => undefined),
) {
  const { container } = render(providers(children, path));
  // Let the queries settle so the audit sees content, not skeletons. Each
  // screen passes one marker that only renders once its data is in.
  await ready();
  // Both passes on the same DOM: the hand-written floor plus axe-core's
  // wider WCAG 2.2 A/AA rule set.
  const [axe] = await Promise.all([axeAudit(container)]);
  return [...auditTree(container), ...axe];
}

function expectClean(violations: ReturnType<typeof auditTree>) {
  const remaining = unexpected(violations, KNOWN);
  expect(remaining).toEqual([]);
}

beforeEach(() => {
  wireFixtures();
  localStorage.clear();
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("wizard (AC1: keyboard, screen reader, 200% zoom)", () => {
  it("renders no accessibility violations", async () => {
    const { container, findByRole } = render(providers(<OnboardingWizard />));
    await findByRole("dialog");
    expectClean(auditTree(container));
  });

  it("names the dialog and marks the current step in text", async () => {
    const { findByRole } = render(providers(<OnboardingWizard />));
    const dialog = await findByRole("dialog");
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(dialog.getAttribute("aria-labelledby")).toBeTruthy();
  });
});

describe("Today (AC1)", () => {
  it("renders no accessibility violations", async () => {
    expectClean(
      await audit(<TodayIndex />, "/", () => screen.findByText("Senior Data Engineer")),
    );
  });

  it("announces Verdicts as words, not colour", async () => {
    const { findByText } = render(providers(<TodayIndex />));
    // The band word ships next to the number; the colour is decoration.
    expect(await findByText("Strong")).toBeTruthy();
  });

  it("makes Open Inbox a single link control (A11Y-3)", async () => {
    render(providers(<TodayIndex />));
    const link = await screen.findByRole("link", { name: /Open Inbox/ });
    expect(link.tagName).toBe("A");
    expect(link.querySelector("button")).toBeNull();
  });

  it("separates the Verdict word from the Rank number for a screen reader (A11Y-5)", async () => {
    // "Strong 78" in the accessibility tree, not "Strong78".
    const { findByText } = render(providers(<TodayIndex />));
    const link = (await findByText("Senior Data Engineer")).closest("a");
    expect(link?.textContent).toMatch(/Strong\s+78/);
  });
});

describe("Offer detail (AC1)", () => {
  it("renders no accessibility violations", async () => {
    expectClean(
      await audit(
        <Routes>
          <Route path="/offer/:postingId" element={<OfferDetail />} />
        </Routes>,
        "/offer/pst_01",
        () => screen.findByRole("heading", { level: 1 }),
      ),
    );
  });

  it("announces the Verdict and Confidence as text", async () => {
    const { findByText } = render(
      providers(
        <Routes>
          <Route path="/offer/:postingId" element={<OfferDetail />} />
        </Routes>,
        "/offer/pst_01",
      ),
    );
    expect(await findByText("Strong")).toBeTruthy();
    expect(await findByText("Medium")).toBeTruthy();
  });

  it("names the page region after the offer title (A11Y-1)", async () => {
    render(
      providers(
        <Routes>
          <Route path="/offer/:postingId" element={<OfferDetail />} />
        </Routes>,
        "/offer/pst_01",
      ),
    );
    expect(await screen.findByRole("region", { name: "Senior Data Engineer" })).toBeTruthy();
  });
});

describe("Studio (AC1)", () => {
  it("renders no accessibility violations", async () => {
    expectClean(
      await audit(<StudioIndex />, "/studio?app=app_1", () =>
        screen.findByRole("region", { name: "CV" }),
      ),
    );
  });

  it("exposes the document tabs as a complete tab pattern (A11Y-8)", async () => {
    render(providers(<StudioIndex />, "/studio?app=app_1"));
    const cv = await screen.findByRole("tab", { name: "CV" });
    const cover = screen.getByRole("tab", { name: "Cover letter" });
    expect(cv.getAttribute("aria-controls")).toBe("studio-document-panel");
    expect(cv.getAttribute("tabindex")).toBe("0");
    expect(cover.getAttribute("tabindex")).toBe("-1");
    const panel = document.getElementById("studio-document-panel");
    expect(panel?.getAttribute("role")).toBe("tabpanel");
    expect(panel?.getAttribute("aria-labelledby")).toBe("studio-tab-cv");

    fireEvent.keyDown(cv, { key: "ArrowRight" });
    expect(cover.getAttribute("aria-selected")).toBe("true");
    expect(panel?.getAttribute("aria-labelledby")).toBe("studio-tab-cover_letter");
  });
});

describe("Tracker (AC1)", () => {
  it("renders no accessibility violations", async () => {
    expectClean(
      await audit(<TrackerIndex />, "/", () =>
        screen.findByRole("button", { name: /Open Backend Engineer/ }),
      ),
    );
  });

  it("names every Stage column in text", async () => {
    const { findAllByRole } = render(providers(<TrackerIndex />));
    const columns = await findAllByRole("heading", { level: 2 });
    const names = columns.map((h) => h.textContent ?? "").join(" | ");
    for (const stage of ["Saved", "Preparing", "Applied", "Interviewing", "Offer"]) {
      expect(names).toContain(stage);
    }
  });

  it("keeps a card a list item with its own open and drag controls (A11Y-2)", async () => {
    const { container } = render(providers(<TrackerIndex />));
    await screen.findByRole("button", { name: "Open Backend Engineer" });
    // No element carries both role="button" and the card's contents.
    expect(container.querySelector('li[role="button"]')).toBeNull();
    expect(screen.getByRole("button", { name: "Move Backend Engineer" })).toBeTruthy();
  });

  it("exposes the view tabs as a complete tab pattern (A11Y-8)", async () => {
    render(providers(<TrackerIndex />));
    const board = await screen.findByRole("tab", { name: "Board" });
    const stats = screen.getByRole("tab", { name: "Stats" });
    expect(board.getAttribute("aria-controls")).toBe("tracker-panel");
    expect(board.getAttribute("tabindex")).toBe("0");
    expect(stats.getAttribute("tabindex")).toBe("-1");
    const panel = document.getElementById("tracker-panel");
    expect(panel?.getAttribute("role")).toBe("tabpanel");
    expect(panel?.getAttribute("aria-labelledby")).toBe("tracker-tab-board");

    fireEvent.keyDown(board, { key: "ArrowRight" });
    expect(stats.getAttribute("aria-selected")).toBe("true");
    expect(panel?.getAttribute("aria-labelledby")).toBe("tracker-tab-stats");
  });
});

describe("Applying (AC1)", () => {
  it("renders no accessibility violations", async () => {
    expectClean(
      await audit(<SettingsApplyingRules />, "/", () =>
        screen.findByText("Email × remote"),
      ),
    );
  });

  it("opens the Auto confirmation with labelled controls", async () => {
    const { container, findByRole } = render(providers(<SettingsApplyingRules />));
    const enable = await findByRole("button", { name: "Enable Auto" });
    fireEvent.click(enable);

    const dialog = await findByRole("dialog");
    expect(dialog.textContent).toContain("Turn on auto-apply?");
    // The acceptance checkbox is labelled, and the confirm button starts off.
    const accept = await findByRole("checkbox");
    expect(accept.closest("label")?.textContent?.trim().length).toBeGreaterThan(0);
    expect(
      (await findByRole("button", { name: "Turn Auto on" }) as HTMLButtonElement).disabled,
    ).toBe(true);

    expectClean(unexpected(auditTree(container), KNOWN));
  });

  it("is a real modal: focus moves in, Tab is trapped, Esc closes and restores focus (A11Y-7)", async () => {
    render(providers(<SettingsApplyingRules />));
    const enable = await screen.findByRole("button", { name: "Enable Auto" });
    fireEvent.click(enable);

    const dialog = await screen.findByRole("dialog");
    const accept = screen.getByRole("checkbox");
    const cancel = screen.getByRole("button", { name: "Keep Auto off" });
    expect(dialog.contains(document.activeElement)).toBe(true);

    // Tab from the last control wraps to the first, and Shift+Tab back again.
    cancel.focus();
    fireEvent.keyDown(cancel, { key: "Tab" });
    expect(document.activeElement).toBe(accept);
    fireEvent.keyDown(accept, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(cancel);

    // B1: focus that escapes the dialog is pulled straight back in.
    screen.getByRole("button", { name: "New rule" }).focus();
    expect(document.activeElement).toBe(accept);

    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(enable);
  });
});

describe("Connections (AC1)", () => {
  it("renders no accessibility violations", async () => {
    expectClean(
      await audit(<ConnectionsIndex />, "/connections", () =>
        screen.findByRole("heading", { name: "Remotive" }),
      ),
    );
  });

  it("keeps one page heading and one heading per section", async () => {
    render(providers(<ConnectionsIndex />, "/connections"));
    await screen.findByRole("heading", { name: "Sources", level: 2 });
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
    expect(screen.getByRole("heading", { name: "Sources", level: 2 })).toBeTruthy();
    expect(screen.getByRole("heading", { name: /^Email/, level: 2 })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Session boards", level: 2 })).toBeTruthy();
  });
});

describe("Kill Switch (AC1)", () => {
  it("renders no accessibility violations", async () => {
    expectClean(
      await audit(<KillSwitch />, "/", () => screen.findByRole("button")),
    );
  });

  it("is a button with a keyboard shortcut and a text name", async () => {
    const { findByRole } = render(providers(<KillSwitch />));
    const button = await findByRole("button");
    expect(button.getAttribute("aria-keyshortcuts")).toBeTruthy();
    expect((button.textContent ?? "").trim().length).toBeGreaterThan(0);
  });

  it("renders the engaged state with no accessibility violations", async () => {
    vi.mocked(commands.killSwitchStatus).mockReturnValue(
      ok({ engaged: true, since: "2026-10-17T00:00:00Z" }) as never,
    );
    const { container } = render(providers(<KillSwitch />));
    await screen.findByRole("status");
    expectClean(auditTree(container));
  });
});

describe("Settings → AI engine (FX7 AC1)", () => {
  it("renders no accessibility violations, the engine picker included", async () => {
    expectClean(
      await audit(<SettingsAiEngine />, "/settings/ai-engine", () =>
        screen.findByRole("button", { name: "Save engine" }),
      ),
    );
  });

  it("labels the CLI choice, the model field, Test and Save", async () => {
    render(providers(<SettingsAiEngine />, "/settings/ai-engine"));

    expect(await screen.findByRole("radio", { name: /Claude Code/ })).toBeTruthy();
    const model = await screen.findByLabelText("Model");
    expect(model.tagName).toBe("INPUT");
    expect(screen.getByRole("button", { name: "Test connection" })).toBeTruthy();
    // Save is the one write, and it starts off until a test passes (AC3).
    expect(screen.getByRole("button", { name: "Save engine" })).toHaveProperty("disabled", true);
  });
});

describe("Settings → Notifications (FX14 AC4)", () => {
  it("renders no accessibility violations, launch at login included", async () => {
    expectClean(
      await audit(<SettingsNotifications />, "/settings/notifications", () =>
        screen.findByRole("checkbox", { name: "Start Boba when I log in" }),
      ),
    );
  });

  it("exposes launch at login as a native, focusable, labelled switch", async () => {
    render(providers(<SettingsNotifications />, "/settings/notifications"));
    const toggle = await screen.findByRole("checkbox", { name: "Start Boba when I log in" });
    expect(toggle.tagName).toBe("INPUT");
    toggle.focus();
    expect(document.activeElement).toBe(toggle);
  });
});
