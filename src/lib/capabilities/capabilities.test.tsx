import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { type CapabilitySnapshot, commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import { type CapabilityId, capabilityOf, useCapability } from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    capabilitiesSnapshot: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const state = (id: string, available: boolean, reason: "no_agent" | "cli_unsupported" | null) => ({
  id,
  available,
  reason,
});

const limitedSnapshot = (): CapabilitySnapshot => ({
  limited: true,
  capabilities: [
    state("sources", true, null),
    state("dedup", true, null),
    state("scoring", true, null),
    state("tracker", true, null),
    state("applications_manual", true, null),
    state("profile_edit", true, null),
    state("profile_extraction", false, "no_agent"),
    state("tailoring", false, "no_agent"),
    state("email_classification", false, "no_agent"),
  ],
});

const unsupportedSnapshot = (): CapabilitySnapshot => ({
  limited: false,
  capabilities: [
    state("sources", true, null),
    state("profile_extraction", true, null),
    state("tailoring", false, "cli_unsupported"),
  ],
});

/** Renders one capability's access, the way a feature slice reads it (AC4). */
function Probe({ id }: { id: string }) {
  const access = useCapability(id as CapabilityId);
  return (
    <output data-testid="access">
      {JSON.stringify({
        available: access.available,
        reason: access.reason,
        limited: access.limited,
        isLoading: access.isLoading,
      })}
    </output>
  );
}

function renderProbe(id: string) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return render(<Probe id={id} />, { wrapper });
}

const access = (): Record<string, unknown> =>
  JSON.parse(screen.getByTestId("access").textContent ?? "{}") as Record<string, unknown>;

beforeEach(() => {
  vi.mocked(commands.capabilitiesSnapshot).mockReset();
});

describe("useCapability — the shared gate (FR-4, AC4)", () => {
  it("keeps deterministic work available in Limited mode", async () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() => ok(limitedSnapshot()));
    renderProbe("tracker");

    await waitFor(() => expect(access().isLoading).toBe(false));
    expect(access()).toMatchObject({ available: true, reason: null, limited: true });
  });

  it("reports an AI-only action off with the Connect-an-agent reason", async () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() => ok(limitedSnapshot()));
    renderProbe("tailoring");

    await waitFor(() => expect(access().isLoading).toBe(false));
    expect(access()).toMatchObject({
      available: false,
      reason: "no_agent",
      limited: true,
    });
  });

  it("reports the lockdown reason when the chosen CLI cannot run the task", async () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() => ok(unsupportedSnapshot()));
    renderProbe("tailoring");

    await waitFor(() => expect(access().isLoading).toBe(false));
    expect(access()).toMatchObject({
      available: false,
      reason: "cli_unsupported",
      limited: false,
    });
  });

  it("stays closed while the snapshot loads, so no control fails later", () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(
      () => new Promise<never>(() => undefined),
    );
    renderProbe("tailoring");

    expect(access()).toMatchObject({ available: false, reason: null, isLoading: true });
  });

  it("stays closed when the snapshot cannot be read", async () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() =>
      Promise.resolve({
        status: "error" as const,
        error: { code: "internal" as const, message_key: "errors.unknown" },
      }),
    );
    renderProbe("tailoring");

    await waitFor(() => expect(access().isLoading).toBe(false));
    expect(access()).toMatchObject({ available: false, reason: null });
  });

  it("fails closed for a capability the core does not report", () => {
    const snapshot = unsupportedSnapshot();
    expect(capabilityOf(snapshot, "not_a_capability")).toEqual({
      id: "not_a_capability",
      available: false,
      reason: null,
    });
  });
});
