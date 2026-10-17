import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Button } from "@/components/ui/button";
import { type CapabilitySnapshot, commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import { Gated } from "./index";

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

const snapshot = (over: Partial<CapabilitySnapshot> = {}): CapabilitySnapshot => ({
  limited: false,
  capabilities: [state("tailoring", true, null)],
  ...over,
});

function renderGated(children: ReactNode = <Button>Tailor</Button>) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={["/studio"]}>
        <Routes>
          <Route path="/studio" element={<Gated capability="tailoring">{children}</Gated>} />
          <Route path="/onboarding" element={<p>agent setup</p>} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.capabilitiesSnapshot).mockReset();
});

describe("Gated — the shared disabled state (AC2, AC4)", () => {
  it("leaves the action working when the capability is available", async () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() => ok(snapshot()));
    renderGated();

    await waitFor(() => expect(screen.getByRole("button", { name: "Tailor" })).toBeEnabled());
    expect(screen.queryByText(/AI features are off/)).toBeNull();
  });

  it("disables the action and offers Connect an agent in Limited mode", async () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() =>
      ok(snapshot({ limited: true, capabilities: [state("tailoring", false, "no_agent")] })),
    );
    renderGated();

    expect(await screen.findByText("AI features are off — no Agent CLI connected.")).toBeTruthy();
    const tailor = screen.getByRole("button", { name: "Tailor" });
    expect(tailor).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Connect an agent" }));
    expect(await screen.findByText("agent setup")).toBeTruthy();
  });

  it("gives the lockdown reason when the chosen CLI cannot run the task", async () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(() =>
      ok(snapshot({ capabilities: [state("tailoring", false, "cli_unsupported")] })),
    );
    renderGated();

    expect(await screen.findByText("This action needs a fully locked-down CLI.")).toBeTruthy();
    const tailor = screen.getByRole("button", { name: "Tailor" });
    expect(tailor).toBeDisabled();
    // The fix is another CLI, never an override (EXPERIENCE §9.15).
    expect(screen.getByRole("button", { name: "Change CLI" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Connect an agent" })).toBeNull();
  });

  it("keeps the action disabled while the snapshot is unknown", () => {
    vi.mocked(commands.capabilitiesSnapshot).mockImplementation(
      () => new Promise<never>(() => undefined),
    );
    renderGated();

    const tailor = screen.getByRole("button", { name: "Tailor" });
    expect(tailor).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Connect an agent" })).toBeNull();
  });
});
