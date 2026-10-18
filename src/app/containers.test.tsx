import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SETTINGS_PAGES } from "@/features/settings";
import { TODAY_PANELS } from "@/features/today";
import "@/lib/i18n";
import { routes } from "./routes";

/**
 * FX18 — the container half of the "built but never mounted" guard.
 *
 * The beta.2 stub sweep found six leaves whose own tests passed because they
 * rendered the leaf alone while no route or container ever mounted it. This
 * file renders every container for real (routes, the Today panel list, the
 * Settings page list) in the state the app boots into — a locked store and
 * Limited mode — and asserts each registered leaf's heading. Mounting is not
 * asserted from the registry alone: the registry is what the container renders,
 * and a leaf that is listed but never renders fails here.
 *
 * Two reads are answered with an empty success instead of `locked`: the Matches
 * panel and the run report hide their heading while their read fails, so the
 * locked answer would make a correct mount look unmounted. Everything else
 * keeps the locked answer.
 */

vi.mock("@/lib/ipc/bindings", () => {
  const locked = { code: "locked", message_key: "errors.store.locked" };
  const error = () => Promise.resolve({ status: "error", error: locked });
  const ok = (data: unknown) => Promise.resolve({ status: "ok", data });

  const overrides: Record<string, () => Promise<unknown>> = {
    offersInbox: () => ok([]),
    runReportLatest: () =>
      ok({
        run_id: "run_0",
        retry_of: null,
        started_at: "2026-10-18T08:00:00Z",
        finished_at: "2026-10-18T08:00:01Z",
        duration_ms: 1000,
        sources: [],
        queried: 0,
        fetched: 0,
        new: 0,
        duplicates: 0,
        vetoed: 0,
        ranked: 0,
        agent_tasks_used: 0,
        errors: [],
        partial: false,
      }),
    capabilitiesSnapshot: () => ok({ limited: true, agent: null, capabilities: [] }),
  };

  const commands = new Proxy(
    {},
    {
      get(_target, key) {
        if (typeof key !== "string") return undefined;
        return overrides[key] ?? error;
      },
    },
  );

  const events = new Proxy(
    {},
    {
      get() {
        return { listen: () => Promise.resolve(() => {}) };
      },
    },
  );

  return { commands, events };
});

function renderAt(path: string) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const router = createMemoryRouter(routes, { initialEntries: [path] });
  return render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  localStorage.clear();
  // The core cannot answer while the store is locked, so the launch mirror
  // decides; without it the first-run gate would send every route to setup.
  localStorage.setItem("boba.onboarding.done", "1");
});

/**
 * The routed screen's own DOM. The shell's topbar also renders an `h1` with the
 * page title, so an unscoped query would pass on the topbar alone — the very
 * class of false green this file exists to remove. Full-window routes (setup,
 * unlock) have no `main` and are queried as is.
 */
function pageScope() {
  const main = document.getElementById("main");
  return within(main ?? document.body);
}

/** One entry per top-level route: the heading its real screen renders. */
const ROUTE_HEADINGS: readonly { path: string; heading: string; onboarded?: boolean }[] = [
  { path: "/", heading: "Today" },
  { path: "/inbox", heading: "Inbox" },
  { path: "/studio", heading: "Pick an application" },
  { path: "/tracker", heading: "Tracker" },
  { path: "/profile", heading: "Profile" },
  { path: "/connections", heading: "Connections" },
  { path: "/settings", heading: "Settings" },
  { path: "/unlock", heading: "Unlock Boba" },
  { path: "/onboarding", heading: "Boba setup", onboarded: false },
];

describe("routes mount their real screen", () => {
  for (const route of ROUTE_HEADINGS) {
    it(`${route.path} renders "${route.heading}"`, async () => {
      // Setup only mounts on a machine that has not finished it.
      if (route.onboarded === false) localStorage.removeItem("boba.onboarding.done");
      renderAt(route.path);

      expect(
        await pageScope().findByRole("heading", { name: route.heading, level: 1 }),
      ).toBeTruthy();
    });
  }

  it("/offer with no id renders its empty state, not a blank page", async () => {
    renderAt("/offer");

    expect(await screen.findByText("No Offer selected")).toBeTruthy();
  });

  it("an unknown path renders the catch-all screen", async () => {
    renderAt("/no-such-screen");

    expect(await screen.findByText("Nothing here yet")).toBeTruthy();
  });
});

/**
 * One entry per registered Today panel, keyed by its `TODAY_PANELS` id. A new
 * panel without a heading here (or a heading that stops rendering) fails.
 */
const PANEL_HEADINGS: Record<string, string> = {
  matches: "Matches",
  "needs-you": "Needs you",
  outbox: "Outbox",
  "run-report": "Last search run",
};

describe("Today mounts every registered panel", () => {
  it("registers every panel in this table, and nothing else", () => {
    expect(TODAY_PANELS.map((panel) => panel.id).sort()).toEqual(
      Object.keys(PANEL_HEADINGS).sort(),
    );
  });

  it("renders each panel's heading on /", async () => {
    renderAt("/");

    for (const panel of TODAY_PANELS) {
      expect(
        await pageScope().findByRole("heading", { name: PANEL_HEADINGS[panel.id] }),
      ).toBeTruthy();
    }
  });
});

/**
 * One entry per registered Settings page, keyed by its `SETTINGS_PAGES` id.
 * The heading id proves the real page rendered, not the container's `h1`.
 */
const PAGE_HEADINGS: Record<string, { id: string; name: string }> = {
  preferences: { id: "settings-preferences-title", name: "Preferences" },
  "privacy/sessions": { id: "settings-sessions-title", name: "Sessions" },
  "privacy/audit": { id: "settings-audit-title", name: "Audit Log" },
  "privacy/backup": { id: "settings-backup-title", name: "Backup and restore" },
  "privacy/disclosures": { id: "disclosures-title", name: "Disclosures" },
  "privacy/export": { id: "settings-export-title", name: "Export your data" },
  "privacy/network": { id: "settings-network-title", name: "Network activity" },
  applying: { id: "settings-applying-title", name: "Apply" },
  "applying/session": { id: "settings-session-title", name: "Session Board Auto" },
  "ai-engine": { id: "settings-ai-engine-title", name: "AI engine" },
  notifications: { id: "settings-notifications-title", name: "Notifications" },
  scoring: { id: "settings-scoring-title", name: "Scoring" },
};

describe("Settings mounts every registered page", () => {
  it("registers every page in this table, and nothing else", () => {
    expect(SETTINGS_PAGES.map((page) => page.id).sort()).toEqual(Object.keys(PAGE_HEADINGS).sort());
  });

  for (const page of Object.keys(PAGE_HEADINGS)) {
    it(`/settings/${page} renders the real page`, async () => {
      renderAt(`/settings/${page}`);

      const scope = pageScope();
      const heading = await scope.findByRole("heading", { name: PAGE_HEADINGS[page].name });
      expect(heading.id).toBe(PAGE_HEADINGS[page].id);
      // The container keeps its own heading and nav around the page.
      expect(scope.getByRole("heading", { name: "Settings", level: 1 })).toBeTruthy();
    });
  }
});
