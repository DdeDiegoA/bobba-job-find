import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, RunReportProgressed, RunReportView } from "@/lib/ipc/bindings";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import TodayRunReport from "./index";
import { RunProgress } from "./run-progress";
import { formatDuration } from "./use-run-report";

/** The callback the component registered for the progress event. */
let onProgress: ((event: { payload: RunReportProgressed }) => void) | null = null;

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    runReportLatest: vi.fn(),
    runReportRetry: vi.fn(),
  },
  events: {
    runReportProgressed: {
      listen: vi.fn((cb: (event: { payload: RunReportProgressed }) => void) => {
        onProgress = cb;
        return Promise.resolve(() => {
          onProgress = null;
        });
      }),
    },
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const failed = (error: AppError) => Promise.resolve({ status: "error" as const, error });

const report = (over: Partial<RunReportView> = {}): RunReportView => ({
  run_id: "run_1",
  retry_of: null,
  started_at: "2026-10-17T08:00:00Z",
  finished_at: "2026-10-17T08:01:00Z",
  duration_ms: 60_000,
  sources: [
    {
      source_id: "remotive",
      name: "Remotive",
      outcome: "ok",
      fetched: 12,
      new: 9,
      duplicates: 3,
      error: null,
    },
    {
      source_id: "hn",
      name: "Hacker News",
      outcome: "failed",
      fetched: 0,
      new: 0,
      duplicates: 0,
      error: "HTTP 503",
    },
    {
      source_id: "himalayas",
      name: "Himalayas",
      outcome: "ok",
      fetched: 5,
      new: 2,
      duplicates: 3,
      error: null,
    },
  ],
  queried: 3,
  fetched: 17,
  new: 11,
  duplicates: 6,
  vetoed: 1,
  ranked: 10,
  agent_tasks_used: 4,
  errors: [],
  partial: true,
  ...over,
});

function renderPanel() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <TodayRunReport />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.runReportLatest).mockReset();
  vi.mocked(commands.runReportRetry).mockReset();
  onProgress = null;
});

describe("the run report panel", () => {
  it("shows the totals and every Source of the last run", async () => {
    vi.mocked(commands.runReportLatest).mockReturnValue(ok(report()));

    renderPanel();

    expect(await screen.findByText("17")).toBeTruthy();
    expect(screen.getByText("11")).toBeTruthy();
    expect(screen.getByText("6")).toBeTruthy();
    expect(screen.getByText("Remotive")).toBeTruthy();
    expect(screen.getByText("Himalayas")).toBeTruthy();
  });

  it("explains an empty state before the first run", async () => {
    vi.mocked(commands.runReportLatest).mockReturnValue(ok(null));

    renderPanel();

    expect(await screen.findByText(/no run yet/i)).toBeTruthy();
  });

  it("shows a read error instead of pretending there was no run", async () => {
    vi.mocked(commands.runReportLatest).mockReturnValue(
      failed({ code: "internal", message_key: "errors.run_report.store" }),
    );

    renderPanel();

    expect(await screen.findByText(/could not save/i)).toBeTruthy();
  });

  it("lists the failed Sources and retries them from the banner", async () => {
    vi.mocked(commands.runReportLatest).mockReturnValue(ok(report()));
    vi.mocked(commands.runReportRetry).mockReturnValue(ok(null));

    renderPanel();

    const banner = await screen.findByRole("alert");
    expect(banner.textContent).toContain("Hacker News");
    expect(banner.textContent).toContain("HTTP 503");
    fireEvent.click(screen.getByRole("button", { name: /retry/i }));
    await waitFor(() => expect(commands.runReportRetry).toHaveBeenCalledTimes(1));
  });

  it("keeps the banner out of a run that reached every Source", async () => {
    vi.mocked(commands.runReportLatest).mockReturnValue(
      ok(
        report({
          partial: false,
          sources: [
            {
              source_id: "remotive",
              name: "Remotive",
              outcome: "ok",
              fetched: 1,
              new: 1,
              duplicates: 0,
              error: null,
            },
          ],
        }),
      ),
    );

    renderPanel();

    await screen.findByText("Remotive");
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("the topbar progress", () => {
  it("shows the Source being queried as the run works", async () => {
    render(
      <QueryClientProvider client={new QueryClient()}>
        <RunProgress />
      </QueryClientProvider>,
    );
    await waitFor(() => expect(onProgress).not.toBeNull());

    act(() => {
      onProgress?.({
        payload: {
          run_id: "run_1",
          phase: "source",
          done: 2,
          total: 7,
          source_id: "remotive",
          source_name: "Remotive",
          outcome: "ok",
          partial: null,
        },
      });
    });

    const status = await screen.findByRole("status");
    expect(status.textContent).toContain("Remotive");
    expect(status.textContent).toContain("2/7");
  });

  it("renders nothing between runs", () => {
    render(
      <QueryClientProvider client={new QueryClient()}>
        <RunProgress />
      </QueryClientProvider>,
    );
    expect(screen.queryByRole("status")).toBeNull();
  });
});

describe("duration formatting", () => {
  it("reads as minutes and seconds, then hours", () => {
    expect(formatDuration(42_000)).toBe("0:42");
    expect(formatDuration(127_000)).toBe("2:07");
    expect(formatDuration(3_727_000)).toBe("1:02:07");
  });
});
