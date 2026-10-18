// S10.5 AC1 / REL2: WiX (MSI) accepts only a numeric version, but the app
// version is semver and a beta carries `1.0.0-beta.1` (tauri-utils 2.10 and
// above reject the pre-release suffix for the msi target). The mapping below
// is what release.yml feeds to `bundle.windows.wix.version`.
import { describe, expect, it } from "vitest";
import { wixConfig, wixVersion } from "./wix-version.mjs";

describe("wixVersion", () => {
  it("maps a beta tag onto the build field", () => {
    expect(wixVersion("v1.0.0-beta.1")).toEqual({ ok: true, version: "1.0.0.1" });
  });

  it("keeps counting betas upward", () => {
    expect(wixVersion("v1.0.0-beta.12")).toEqual({ ok: true, version: "1.0.0.12" });
  });

  it("maps any pre-release to the last numeric identifier", () => {
    expect(wixVersion("v2.3.4-rc.7")).toEqual({ ok: true, version: "2.3.4.7" });
  });

  it("falls back to build 1 when the pre-release has no number", () => {
    expect(wixVersion("v1.0.0-beta")).toEqual({ ok: true, version: "1.0.0.1" });
  });

  it("gives a stable release the highest build so it upgrades its betas", () => {
    expect(wixVersion("v1.0.0")).toEqual({ ok: true, version: "1.0.0.65535" });
  });

  it("sorts a stable release above the betas of the same version", () => {
    const beta = wixVersion("v1.0.0-beta.9").version.split(".").map(Number);
    const stable = wixVersion("v1.0.0").version.split(".").map(Number);
    expect(stable.at(-1)).toBeGreaterThan(beta.at(-1));
  });

  it("rejects a tag that is not v-prefixed semver", () => {
    const result = wixVersion("1.0.0");
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/v\d/);
  });

  it("rejects a pre-release number the MSI cannot carry", () => {
    const result = wixVersion("v1.0.0-beta.65536");
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/65535/);
  });

  it("rejects a major or minor field above 255", () => {
    expect(wixVersion("v256.0.0").ok).toBe(false);
    expect(wixVersion("v1.256.0").ok).toBe(false);
  });

  it("rejects a patch field above 65535", () => {
    const result = wixVersion("v1.0.65536");
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/65535/);
  });
});

describe("wixConfig", () => {
  it("is the platform config fragment Tauri merges into tauri.conf.json", () => {
    expect(wixConfig("1.0.0.1")).toEqual({
      bundle: { windows: { wix: { version: "1.0.0.1" } } },
    });
  });
});
