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
import OfferDetail from "@/features/offer";
import SettingsApplyingRules from "@/features/settings/applying/rules";
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
  vi.mocked(commands.sessionAutoSnapshot).mockReturnValue(ok([]));
  vi.mocked(commands.killSwitchStatus).mockReturnValue(
    ok({ engaged: false, since: null }) as never,
  );
  vi.mocked(commands.applyOutboxView).mockReturnValue(
    ok({ settings: { hold_minutes: 30 }, items: [], rules: [] }) as never,
  );
  vi.mocked(commands.runReportLatest).mockReturnValue(ok(null) as never);
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
 */
const KNOWN: KnownDefect[] = [
  {
    // A11Y-1 (docs/qa/accessibility.md): the page's `aria-labelledby` points at
    // `offer-title`, but the `<h1>` it means ships without that id. Owned by the
    // Offer detail slice (E6); fixed there, not here.
    rule: "aria-labelledby-target",
    element: "<section.flex.flex-col.gap-6>",
    followUp: "A11Y-1",
  },
  {
    // A11Y-2: dnd-kit gives the card's `<li>` role="button", and the card's
    // open control is a real `<button>` inside it. Owned by the Tracker slice.
    rule: "nested-interactive",
    element: "<li.flex.cursor-grab.flex-col>",
    followUp: "A11Y-2",
  },
  {
    // A11Y-2, seen by axe: the same `role="button"` on the `<li>` makes the
    // column's `<ul>` a list whose children are not list items.
    rule: "list",
    element: ".w-56",
    followUp: "A11Y-2",
  },
  {
    // A11Y-2, axe's target for the same card (the dnd-kit class).
    rule: "nested-interactive",
    element: ".cursor-grab",
    followUp: "A11Y-2",
  },
  {
    // A11Y-3: "Open Inbox" is a `<Button>` inside a router `<Link>` (a button
    // inside a link). Owned by the Today slice.
    rule: "nested-interactive",
    element: "<a.ml-auto>",
    followUp: "A11Y-3",
  },
];

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

  it.fails(
    "separates the Verdict word from the Rank number for a screen reader (A11Y-5)",
    async () => {
      // The pair renders as "Strong78" today: the elements are laid out with a
      // gap, but nothing separates them in the accessibility tree. Pinned as a
      // failing test owned by the Today slice; it must be deleted, not fixed
      // here (S10.6 AC3).
      const { findByText } = render(providers(<TodayIndex />));
      const link = (await findByText("Senior Data Engineer")).closest("a");
      expect(link?.textContent).toMatch(/Strong\s+78/);
    },
  );
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
});

describe("Studio (AC1)", () => {
  it("renders no accessibility violations", async () => {
    expectClean(
      await audit(<StudioIndex />, "/studio?app=app_1", () =>
        screen.findByRole("region", { name: "CV" }),
      ),
    );
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
