import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, RssTestView, SourceTileView } from "@/lib/ipc/bindings";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import ConnectionsSources from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    sourcesCatalogue: vi.fn(),
    sourcesTestRss: vi.fn(),
    sourcesAddRss: vi.fn(),
    sourcesRemoveRss: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const failed = (error: AppError) => Promise.resolve({ status: "error" as const, error });

const tile = (over: Partial<SourceTileView> = {}): SourceTileView => ({
  id: "remotive",
  name: "Remotive",
  source_type: "feed",
  markets: ["Global remote"],
  risk_tier: "safe",
  custom: false,
  enabled: true,
  status: "ok",
  last_run: "2026-10-17T06:12:00Z",
  offers_found: 31,
  last_error: null,
  consecutive_failures: 0,
  attribution: null,
  ...over,
});

const probe = (over: Partial<RssTestView> = {}): RssTestView => ({
  id: "rss:jobs-example-com-feed-xml",
  name: "jobs.example.com",
  url: "https://jobs.example.com/feed.xml",
  count: 12,
  sample: [
    {
      title: "Senior Rust Engineer",
      company: "Boba Labs",
      url: "https://jobs.example.com/1",
      posted_at: "2026-10-12T08:00:00+00:00",
    },
  ],
  ...over,
});

function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ConnectionsSources />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.sourcesCatalogue).mockReset().mockReturnValue(ok([]));
  vi.mocked(commands.sourcesTestRss).mockReset();
  vi.mocked(commands.sourcesAddRss).mockReset();
  vi.mocked(commands.sourcesRemoveRss).mockReset();
});

describe("source tiles (AC1)", () => {
  it("shows type, markets, risk tier, last run, offers found, error and status", async () => {
    vi.mocked(commands.sourcesCatalogue).mockReturnValue(
      ok([
        tile(),
        tile({
          id: "rss:jobs-example-com-feed-xml",
          name: "Example board",
          source_type: "custom",
          markets: [],
          risk_tier: "safe",
          custom: true,
          status: "failing",
          last_run: "2026-10-17T07:00:00Z",
          offers_found: 0,
          last_error: "status 503",
          consecutive_failures: 3,
        }),
      ]),
    );
    renderScreen();

    const feed = (await screen.findByRole("heading", { name: "Remotive" })).closest("li");
    expect(within(feed as HTMLElement).getByText("Feed")).toBeTruthy();
    expect(within(feed as HTMLElement).getByText("Global remote")).toBeTruthy();
    expect(within(feed as HTMLElement).getByText("Safe")).toBeTruthy();
    expect(within(feed as HTMLElement).getByText("OK")).toBeTruthy();
    expect(within(feed as HTMLElement).getByText(/31 offers found/)).toBeTruthy();
    expect(within(feed as HTMLElement).getByText(/Last run/)).toBeTruthy();

    const custom = (await screen.findByRole("heading", { name: "Example board" })).closest("li");
    expect(within(custom as HTMLElement).getByText("Custom feed")).toBeTruthy();
    expect(within(custom as HTMLElement).getByText("Failing")).toBeTruthy();
    expect(within(custom as HTMLElement).getByText(/status 503/)).toBeTruthy();
    expect(within(custom as HTMLElement).getByText(/3 failures in a row/)).toBeTruthy();
    expect(within(custom as HTMLElement).getByText(/0 offers found/)).toBeTruthy();
  });

  it("says when a source has never run", async () => {
    vi.mocked(commands.sourcesCatalogue).mockReturnValue(
      ok([tile({ last_run: null, offers_found: 0, status: "ok" })]),
    );
    renderScreen();
    const item = (await screen.findByRole("heading", { name: "Remotive" })).closest("li");
    expect(within(item as HTMLElement).getByText("Not run yet")).toBeTruthy();
  });
});

describe("adding a custom feed (AC2)", () => {
  it("shows the count and a sample before offering to save", async () => {
    vi.mocked(commands.sourcesTestRss).mockReturnValue(ok(probe()));
    vi.mocked(commands.sourcesAddRss).mockReturnValue(
      ok(tile({ id: "rss:jobs-example-com-feed-xml", name: "jobs.example.com", custom: true })),
    );
    renderScreen();

    fireEvent.click(await screen.findByRole("button", { name: /add a custom feed/i }));
    const form = (await screen.findByRole("heading", { name: /custom rss or atom feed/i })).closest(
      "form",
    ) as HTMLElement;

    fireEvent.change(within(form).getByLabelText("Feed URL"), {
      target: { value: "https://jobs.example.com/feed.xml" },
    });
    fireEvent.click(within(form).getByRole("button", { name: /test fetch/i }));

    await waitFor(() =>
      expect(commands.sourcesTestRss).toHaveBeenCalledWith(
        "https://jobs.example.com/feed.xml",
        null,
      ),
    );
    expect(await within(form).findByText(/12 items found/)).toBeTruthy();
    expect(within(form).getByText(/Senior Rust Engineer/)).toBeTruthy();
    expect(within(form).getByText(/Boba Labs/)).toBeTruthy();
    // Nothing is saved until the user accepts the preview.
    expect(commands.sourcesAddRss).not.toHaveBeenCalled();

    fireEvent.click(within(form).getByRole("button", { name: /save feed/i }));
    await waitFor(() =>
      expect(commands.sourcesAddRss).toHaveBeenCalledWith(
        "https://jobs.example.com/feed.xml",
        null,
      ),
    );
  });

  it("shows the typed error when the probe fails and offers no save", async () => {
    vi.mocked(commands.sourcesTestRss).mockReturnValue(
      failed({
        code: "unavailable",
        message_key: "errors.sources.fetch",
        details: { reason: "status 503" },
      }),
    );
    renderScreen();

    fireEvent.click(await screen.findByRole("button", { name: /add a custom feed/i }));
    const form = (await screen.findByRole("heading", { name: /custom rss or atom feed/i })).closest(
      "form",
    ) as HTMLElement;
    fireEvent.change(within(form).getByLabelText("Feed URL"), {
      target: { value: "https://jobs.example.com/feed.xml" },
    });
    fireEvent.click(within(form).getByRole("button", { name: /test fetch/i }));

    expect(await within(form).findByText(/couldn't fetch that feed/i)).toBeTruthy();
    expect(within(form).queryByRole("button", { name: /save feed/i })).toBeNull();
  });
});

describe("removing a custom feed", () => {
  it("removes only the custom source that asked", async () => {
    vi.mocked(commands.sourcesCatalogue).mockReturnValue(
      ok([tile(), tile({ id: "rss:x", name: "My feed", custom: true, source_type: "custom" })]),
    );
    vi.mocked(commands.sourcesRemoveRss).mockReturnValue(ok(true));
    renderScreen();

    const custom = (await screen.findByRole("heading", { name: "My feed" })).closest("li");
    expect(screen.getAllByRole("button", { name: /remove/i })).toHaveLength(1);
    fireEvent.click(within(custom as HTMLElement).getByRole("button", { name: /remove/i }));

    await waitFor(() => expect(commands.sourcesRemoveRss).toHaveBeenCalledWith("rss:x"));
  });
});
