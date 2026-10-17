import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type EmbeddingStatus } from "@/lib/ipc/bindings";
import "@/lib/i18n";
import SemanticMatch from "./embeddings";

vi.mock("@/lib/ipc/bindings", () => ({
  commands: {
    embeddingsStatus: vi.fn(),
    embeddingsOptIn: vi.fn(),
    embeddingsOptOut: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const status = (over: Partial<EmbeddingStatus> = {}): EmbeddingStatus => ({
  available: true,
  opted_in: false,
  installed: false,
  model_id: "intfloat/multilingual-e5-small",
  destination: null,
  approximate_bytes: 487_352_505,
  ...over,
});

function renderCard() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SemanticMatch />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(commands.embeddingsStatus).mockReset();
  vi.mocked(commands.embeddingsOptIn).mockReset();
  vi.mocked(commands.embeddingsOptOut).mockReset();
});

describe("Semantic match card (S6.9, AC1/AC4)", () => {
  it("states the download size before anything is fetched", async () => {
    vi.mocked(commands.embeddingsStatus).mockImplementation(() => ok(status()));
    renderCard();

    expect(await screen.findByText(/about 465 MB/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Download the model" })).toBeTruthy();
    expect(commands.embeddingsOptIn).not.toHaveBeenCalled();
  });

  it("downloads only when the user turns it on, then shows where the model is", async () => {
    vi.mocked(commands.embeddingsStatus).mockImplementation(() => ok(status()));
    vi.mocked(commands.embeddingsOptIn).mockImplementation(() =>
      ok(
        status({
          opted_in: true,
          installed: true,
          destination: "/Users/me/Library/Application Support/Boba/models",
        }),
      ),
    );
    renderCard();

    fireEvent.click(await screen.findByRole("button", { name: "Download the model" }));

    expect(await screen.findByText(/Semantic match is on/)).toBeTruthy();
    expect(
      screen.getByText(/\/Users\/me\/Library\/Application Support\/Boba\/models/),
    ).toBeTruthy();
    expect(commands.embeddingsOptIn).toHaveBeenCalledTimes(1);
  });

  it("turns F3 off again and says nothing was kept off the device", async () => {
    vi.mocked(commands.embeddingsStatus)
      .mockImplementationOnce(() =>
        ok(status({ opted_in: true, installed: true, destination: "/models" })),
      )
      .mockImplementation(() => ok(status({ installed: true, destination: "/models" })));
    vi.mocked(commands.embeddingsOptOut).mockImplementation(() =>
      Promise.resolve({ status: "ok" as const, data: null }),
    );
    renderCard();

    fireEvent.click(await screen.findByRole("button", { name: "Turn off" }));

    expect(await screen.findByRole("button", { name: "Download the model" })).toBeTruthy();
    expect(commands.embeddingsOptOut).toHaveBeenCalledTimes(1);
  });

  it("explains that a build without the model simply skips F3", async () => {
    vi.mocked(commands.embeddingsStatus).mockImplementation(() => ok(status({ available: false })));
    renderCard();

    expect(await screen.findByText(/no on-device model/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Download the model" })).toBeNull();
  });
});
