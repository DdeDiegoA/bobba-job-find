import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type SourceTileView } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SourcesStep from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: { sourcesCatalogue: vi.fn(), sourcesAddRss: vi.fn() },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

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

const catalogue = (): SourceTileView[] => [
  tile(),
  tile({
    id: "himalayas",
    name: "Himalayas",
    source_type: "api",
    markets: ["Europe"],
    risk_tier: "safe",
  }),
  tile({
    id: "linkedin",
    name: "LinkedIn",
    source_type: "session_board",
    markets: ["Global remote"],
    risk_tier: "prohibited_automation",
  }),
];

function renderStep() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SourcesStep answers={{}} setAnswer={() => undefined} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.sourcesCatalogue).mockReset().mockReturnValue(ok(catalogue()));
  vi.mocked(commands.sourcesAddRss).mockReset();
});

describe("Sources step — recommended sources (AC1, FR-8)", () => {
  it("pre-selects the Safe Sources for my markets", async () => {
    renderStep();
    const remotive = await screen.findByRole("checkbox", { name: /Remotive/ });
    const himalayas = screen.getByRole("checkbox", { name: /Himalayas/ });
    expect((remotive as HTMLInputElement).checked).toBe(true);
    expect((himalayas as HTMLInputElement).checked).toBe(true);
    // Selecting here is a statement of what is on, not a control: the Sources
    // screen owns the toggle.
    expect((remotive as HTMLInputElement).disabled).toBe(true);
  });

  it("leaves a Prohibited-automation Source out of the selection", async () => {
    renderStep();
    const linkedin = await screen.findByRole("checkbox", { name: /LinkedIn/ });
    expect((linkedin as HTMLInputElement).checked).toBe(false);
  });

  it("badges every Source with its Risk Tier", async () => {
    renderStep();
    const remotive = (await screen.findByRole("checkbox", { name: /Remotive/ })).closest("li");
    expect(remotive).not.toBeNull();
    expect(within(remotive as HTMLElement).getByText("Safe")).toBeTruthy();

    const linkedin = screen.getByRole("checkbox", { name: /LinkedIn/ }).closest("li");
    expect(within(linkedin as HTMLElement).getByText("No automation")).toBeTruthy();
  });

  it("says the list can be changed later in Connections", async () => {
    renderStep();
    expect(await screen.findByText(/change these later in Connections/)).toBeTruthy();
  });

  it("adds a custom feed and shows it with its Safe tier", async () => {
    const added = tile({
      id: "rss:jobs-example-com-feed-xml",
      name: "jobs.example.com",
      source_type: "custom",
      custom: true,
    });
    vi.mocked(commands.sourcesAddRss).mockReturnValue(ok(added));
    renderStep();

    fireEvent.change(await screen.findByLabelText("Feed URL"), {
      target: { value: "https://jobs.example.com/feed.xml" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add feed" }));

    await waitFor(() =>
      expect(commands.sourcesAddRss).toHaveBeenCalledWith(
        "https://jobs.example.com/feed.xml",
        null,
      ),
    );
    expect(await screen.findByRole("checkbox", { name: /jobs\.example\.com/ })).toBeTruthy();
  });
});
