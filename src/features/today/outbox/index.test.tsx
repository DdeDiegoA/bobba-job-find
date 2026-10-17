import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type OutboxItemView, type OutboxView } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import TodayOutbox from "./index";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    applyOutboxView: vi.fn(),
    applyOutboxSetHold: vi.fn(),
    applyOutboxCancel: vi.fn(),
    applyOutboxEdit: vi.fn(),
    applyOutboxActivateRule: vi.fn(),
    applyOutboxSetDryRun: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const item = (over: Partial<OutboxItemView> = {}): OutboxItemView => ({
  id: "out_1",
  application_id: "app_01",
  rule_id: "rule_1",
  rule_name: "Email × remote",
  title: "Backend Engineer",
  company: "Cirrus",
  channel: "email",
  state: "held",
  hold_minutes: 30,
  release_at: "2026-10-17T09:30:00Z",
  minutes_left: 24,
  subject_override: null,
  body_override: null,
  note: null,
  created_at: "2026-10-17T09:00:00Z",
  ...over,
});

const view = (over: Partial<OutboxView> = {}): OutboxView => ({
  settings: { hold_minutes: 30 },
  items: [],
  rules: [],
  ...over,
});

function renderPanel() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <TodayOutbox />
    </QueryClientProvider>,
  );
}

describe("Today outbox panel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(commands.applyOutboxView).mockReturnValue(ok(view()));
  });

  it("shows the empty state when nothing waits (FR-85)", async () => {
    renderPanel();
    expect(await screen.findByText("Nothing is waiting")).toBeTruthy();
  });

  it("shows how long an item still waits and cancels it on request", async () => {
    vi.mocked(commands.applyOutboxView).mockReturnValue(ok(view({ items: [item()] })));
    vi.mocked(commands.applyOutboxCancel).mockReturnValue(
      ok(item({ state: "cancelled", note: "user" })),
    );
    renderPanel();

    expect(await screen.findByText("Waiting 24 min")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    await waitFor(() => expect(commands.applyOutboxCancel).toHaveBeenCalledWith("out_1"));
  });

  it("saves the user's edit during the hold (FR-85)", async () => {
    vi.mocked(commands.applyOutboxView).mockReturnValue(ok(view({ items: [item()] })));
    vi.mocked(commands.applyOutboxEdit).mockReturnValue(ok(item({ subject_override: "Hello" })));
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText("Subject"), {
      target: { value: "Hello" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(commands.applyOutboxEdit).toHaveBeenCalledWith("out_1", "Hello", null),
    );
  });

  it("reports a new rule's dry run and activates it on request (FR-84)", async () => {
    vi.mocked(commands.applyOutboxView).mockReturnValue(
      ok(
        view({
          rules: [
            {
              rule_id: "rule_1",
              rule_name: "Email × remote",
              required: 3,
              seen: 2,
              active: false,
              complete: false,
            },
          ],
        }),
      ),
    );
    vi.mocked(commands.applyOutboxActivateRule).mockReturnValue(
      ok({
        rule_id: "rule_1",
        rule_name: "Email × remote",
        required: 3,
        seen: 3,
        active: true,
        complete: true,
      }),
    );
    renderPanel();

    expect(await screen.findByText("2 of 3 matches reported")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Activate rule" }));

    await waitFor(() => expect(commands.applyOutboxActivateRule).toHaveBeenCalledWith("rule_1"));
  });

  it("saves the hold when the user changes it", async () => {
    vi.mocked(commands.applyOutboxSetHold).mockReturnValue(ok({ hold_minutes: 45 }));
    renderPanel();

    fireEvent.change(await screen.findByRole("spinbutton"), {
      target: { value: "45" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save hold" }));

    await waitFor(() => expect(commands.applyOutboxSetHold).toHaveBeenCalledWith(45));
  });
});
