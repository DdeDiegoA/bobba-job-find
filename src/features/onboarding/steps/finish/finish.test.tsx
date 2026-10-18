import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { MemoryRouter } from "react-router";
import { toast } from "sonner";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import { OnboardingWizard } from "../../wizard";
import { isOnboarded, loadProgress, saveProgress } from "../../wizard/progress";
import FinishStep, { finishSetup } from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    jobsRunNow: vi.fn(),
    applyRulesSnapshot: vi.fn(),
    onboardingComplete: vi.fn(),
    autostartStatus: vi.fn(),
  },
}));

vi.mock("sonner", () => ({ toast: { error: vi.fn() }, Toaster: () => null }));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

/** The core's Auto state; a fresh install has it off (S9.6). */
const rules = (autoEnabled = false) =>
  ok({
    auto_enabled: autoEnabled,
    auto_confirmed: autoEnabled,
    global_daily_cap: 3,
    rules: [],
  });

function withClient(children: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return (
    <QueryClientProvider client={client}>
      <MemoryRouter>{children}</MemoryRouter>
    </QueryClientProvider>
  );
}

function renderStep() {
  return render(withClient(<FinishStep answers={{}} setAnswer={() => undefined} />));
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(commands.jobsRunNow).mockReturnValue(ok({ run_id: "run-1", joined: false }));
  vi.mocked(commands.applyRulesSnapshot).mockReturnValue(rules());
  vi.mocked(commands.autostartStatus).mockResolvedValue({ enabled: false });
  vi.mocked(commands.onboardingComplete).mockReturnValue(
    ok({ completed: true, completed_at: "2026-10-17T09:00:00Z" }),
  );
  localStorage.clear();
});

describe("Finish step — the defaults it hands over (AC4, FR-10)", () => {
  it("shows Apply Mode Review", async () => {
    renderStep();
    expect(await screen.findByText("Review")).toBeTruthy();
    expect(screen.getByText(/Auto-apply is off/)).toBeTruthy();
  });

  it("never lets Auto be turned on from the wizard", () => {
    renderStep();
    expect(screen.queryByRole("radio")).toBeNull();
    expect(screen.queryByRole("checkbox")).toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("says so when an earlier setup left Auto on, and still offers no switch", async () => {
    vi.mocked(commands.applyRulesSnapshot).mockReturnValue(rules(true));
    renderStep();
    expect(await screen.findByText(/Auto \(turned on earlier\)/)).toBeTruthy();
    expect(screen.getByText(/still on from an earlier setup/)).toBeTruthy();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("names the run time and the login state the OS reports", async () => {
    renderStep();
    expect(screen.getByText(/08:00/)).toBeTruthy();
    expect(await screen.findByText("Off")).toBeTruthy();
  });

  it("reads launch at login from the core, not a hardcoded Off", async () => {
    vi.mocked(commands.autostartStatus).mockResolvedValue({ enabled: true });
    renderStep();
    expect(await screen.findByText("On")).toBeTruthy();
    expect(screen.queryByText("Off")).toBeNull();
  });

  it("does not claim a launch state it could not read (FX14 F2)", async () => {
    vi.mocked(commands.autostartStatus).mockRejectedValue({
      code: "locked",
      message_key: "errors.store.locked",
      details: null,
    });
    renderStep();

    expect(
      await screen.findByText("Boba could not read the launch-at-login setting."),
    ).toBeTruthy();
    expect(
      screen.queryByText("Boba opens only when you do. You can turn this on later."),
    ).toBeNull();
  });

  it("links to where launch at login is changed", async () => {
    renderStep();
    const link = await screen.findByRole("link", { name: "Change it in Settings → Notifications" });
    expect(link.getAttribute("href")).toBe("/settings/notifications");
  });
});

describe("Finishing starts the first run (AC4, SM-1)", () => {
  it("starts a Search Run", () => {
    finishSetup();
    expect(commands.jobsRunNow).toHaveBeenCalledTimes(1);
  });

  it("surfaces a run the core refuses, translated (FX14 AC3)", async () => {
    vi.mocked(commands.jobsRunNow).mockReturnValue(
      Promise.resolve({
        status: "error" as const,
        error: { code: "locked", message_key: "errors.store.locked", details: null },
      }),
    );
    finishSetup();
    await waitFor(() =>
      expect(vi.mocked(toast.error)).toHaveBeenCalledWith(
        expect.stringMatching(/can't reach your data/i),
      ),
    );
  });

  it("surfaces a run call that rejects, translated (FX14 AC3)", async () => {
    vi.mocked(commands.jobsRunNow).mockRejectedValue(new Error("bridge down"));
    finishSetup();
    await waitFor(() =>
      expect(vi.mocked(toast.error)).toHaveBeenCalledWith(
        "Boba could not start the first search. You can run it again from Today.",
      ),
    );
  });

  it("is the last step's onFinish: Next on the last step starts the run, marks setup done and hands over to Today", async () => {
    saveProgress({ index: 3, answers: { finish: {} } });
    const onFinished = vi.fn();
    render(
      withClient(
        <OnboardingWizard
          onFinished={onFinished}
          steps={[
            {
              id: "finish",
              titleKey: "onboarding_finish:step.finish.title",
              Component: FinishStep,
              onFinish: finishSetup,
            },
          ]}
        />,
      ),
    );
    fireEvent.click(await screen.findByRole("button", { name: "Resume setup" }));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));

    expect(commands.jobsRunNow).toHaveBeenCalledTimes(1);
    expect(isOnboarded()).toBe(true);
    expect(loadProgress()).toBeNull();
    await waitFor(() => expect(commands.onboardingComplete).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(onFinished).toHaveBeenCalledTimes(1));
  });

  it("still finishes when the run cannot start (locked store)", async () => {
    vi.mocked(commands.jobsRunNow).mockReturnValue(
      Promise.resolve({
        status: "error" as const,
        error: { code: "locked", message_key: "errors.store.locked", details: {} },
      }),
    );
    const onFinished = vi.fn();
    render(
      withClient(
        <OnboardingWizard
          onFinished={onFinished}
          steps={[
            {
              id: "finish",
              titleKey: "onboarding_finish:step.finish.title",
              Component: FinishStep,
              onFinish: finishSetup,
            },
          ]}
        />,
      ),
    );
    fireEvent.click(await screen.findByRole("button", { name: "Next" }));
    expect(isOnboarded()).toBe(true);
    await waitFor(() => expect(onFinished).toHaveBeenCalledTimes(1));
  });
});
