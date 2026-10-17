// S10.5 AC1: the published release carries the installers the story names and
// a `latest.json` the updater can resolve, and its version is the tag.
import { describe, expect, it } from "vitest";
import { checkAssets, checkLatestJson, PLATFORMS } from "./release-manifest.mjs";

const ASSETS = [
  "Boba Job Find_0.3.0_universal.dmg",
  "Boba Job Find_0.3.0_x64-setup.exe",
  "Boba Job Find_0.3.0_x64_en-US.msi",
  "Boba Job Find_0.3.0_amd64.AppImage",
  "boba-job-find_0.3.0_amd64.deb",
  "boba-job-find-0.3.0-1.x86_64.rpm",
  "Boba Job Find.app.tar.gz",
  "Boba Job Find.app.tar.gz.sig",
  "Boba Job Find_0.3.0_x64-setup.exe.sig",
  "Boba Job Find_0.3.0_amd64.AppImage.tar.gz",
  "Boba Job Find_0.3.0_amd64.AppImage.tar.gz.sig",
  "latest.json",
];

const LATEST = {
  version: "0.3.0",
  notes: "Release notes",
  pub_date: "2026-10-17T21:00:00Z",
  platforms: {
    "darwin-aarch64": { signature: "sig", url: "https://x/Boba%20Job%20Find.app.tar.gz" },
    "darwin-x86_64": { signature: "sig", url: "https://x/Boba%20Job%20Find.app.tar.gz" },
    "windows-x86_64": {
      signature: "sig",
      url: "https://x/Boba%20Job%20Find_0.3.0_x64-setup.exe",
    },
    "linux-x86_64": {
      signature: "sig",
      url: "https://x/Boba%20Job%20Find_0.3.0_amd64.AppImage.tar.gz",
    },
  },
};

describe("checkAssets", () => {
  it("accepts the installers the story names", () => {
    expect(checkAssets(ASSETS)).toEqual({ ok: true, errors: [] });
  });

  it("names the missing macOS DMG", () => {
    const result = checkAssets(ASSETS.filter((a) => !a.endsWith(".dmg")));
    expect(result.ok).toBe(false);
    expect(result.errors).toContain("missing the macOS DMG");
  });

  it("names a missing signature file", () => {
    const result = checkAssets(ASSETS.filter((a) => !a.endsWith(".sig")));
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes("updater signature"))).toBe(true);
  });

  it("ignores a plain (non-installer) file", () => {
    expect(checkAssets([...ASSETS, "notes.txt"]).ok).toBe(true);
  });
});

describe("checkLatestJson", () => {
  it("accepts a complete manifest for the tag", () => {
    expect(checkLatestJson({ json: LATEST, tag: "v0.3.0", assets: ASSETS })).toEqual({
      ok: true,
      errors: [],
    });
  });

  it("fails when latest.json is absent", () => {
    const result = checkLatestJson({ json: null, tag: "v0.3.0", assets: ASSETS });
    expect(result.ok).toBe(false);
    expect(result.errors[0]).toMatch(/latest\.json/);
  });

  it("fails when the manifest version is not the tag", () => {
    const result = checkLatestJson({
      json: { ...LATEST, version: "0.2.0" },
      tag: "v0.3.0",
      assets: ASSETS,
    });
    expect(result.ok).toBe(false);
    expect(result.errors).toContain("latest.json version 0.2.0 does not match tag v0.3.0");
  });

  it("fails when a platform entry is missing", () => {
    const json = { ...LATEST, platforms: { ...LATEST.platforms, "windows-x86_64": undefined } };
    const result = checkLatestJson({ json, tag: "v0.3.0", assets: ASSETS });
    expect(result.ok).toBe(false);
    expect(result.errors).toContain("latest.json has no windows-x86_64 entry");
  });

  it("fails when an entry points at an asset that is not published", () => {
    const json = {
      ...LATEST,
      platforms: {
        ...LATEST.platforms,
        "linux-x86_64": { signature: "sig", url: "https://x/missing.AppImage.tar.gz" },
      },
    };
    const result = checkLatestJson({ json, tag: "v0.3.0", assets: ASSETS });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes("missing.AppImage.tar.gz"))).toBe(true);
  });
});

describe("PLATFORMS", () => {
  it("lists every platform the updater resolves", () => {
    expect(PLATFORMS).toEqual([
      "darwin-aarch64",
      "darwin-x86_64",
      "windows-x86_64",
      "linux-x86_64",
    ]);
  });
});
