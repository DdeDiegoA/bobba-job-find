import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import { isOnboarded, markOnboarded } from "./progress";
import { useOnboardingGate } from "./use-onboarding";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: { onboardingStatus: vi.fn() },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

/** The gate as a word, so the render is what the assertions read. */
function Probe() {
  const gate = useOnboardingGate();
  return <p>{gate === null ? "loading" : gate ? "today" : "onboarding"}</p>;
}

function renderGate() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Probe />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
});

describe("first-launch gate — the core is the source of truth (AC1, S3.8)", () => {
  it("reads the core's completion and mirrors it locally", async () => {
    vi.mocked(commands.onboardingStatus).mockReturnValue(
      ok({ completed: true, completed_at: "2026-10-17T09:00:00Z" }),
    );
    renderGate();
    expect(await screen.findByText("today")).toBeTruthy();
    expect(isOnboarded()).toBe(true);
  });

  it("runs setup again when the core says it was never done (a reset wiped it)", async () => {
    markOnboarded();
    vi.mocked(commands.onboardingStatus).mockReturnValue(
      ok({ completed: false, completed_at: null }),
    );
    renderGate();
    expect(await screen.findByText("onboarding")).toBeTruthy();
    await waitFor(() => expect(isOnboarded()).toBe(false));
  });

  it("keeps this machine's answer when a locked store cannot answer", async () => {
    markOnboarded();
    vi.mocked(commands.onboardingStatus).mockReturnValue(
      Promise.resolve({
        status: "error" as const,
        error: { code: "locked", message_key: "errors.store.locked", details: {} },
      }),
    );
    renderGate();
    expect(await screen.findByText("today")).toBeTruthy();
  });

  it("renders nothing until the core answers", () => {
    vi.mocked(commands.onboardingStatus).mockReturnValue(new Promise(() => undefined));
    const { container } = renderGate();
    expect(container.textContent).toBe("loading");
  });
});
