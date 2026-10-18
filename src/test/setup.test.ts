import { getConfig } from "@testing-library/react";
import { describe, expect, it } from "vitest";

describe("Testing Library setup (FX19)", () => {
  it("raises the async query timeout so load cannot flip a pass to a fail", () => {
    expect(getConfig().asyncUtilTimeout).toBe(5000);
  });
});
