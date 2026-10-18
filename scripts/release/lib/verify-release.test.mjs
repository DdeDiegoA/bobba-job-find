// FX21: verification runs twice in the publish job — while the release is a
// draft (url form + asset names only) and after publishing (not a draft, and
// every public url resolves). The draft pass must never demand "published".
import { describe, expect, it, vi } from "vitest";
import { publicDownloadUrl } from "./latest-json.mjs";
import { verifyRelease } from "./verify-release.mjs";

const REPO = "owner/repo";
const TAG = "v0.3.0";
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

const url = (name) => publicDownloadUrl({ repo: REPO, tag: TAG, name });
const LATEST = {
  version: "0.3.0",
  platforms: {
    "darwin-aarch64": { signature: "sig", url: url("Boba Job Find.app.tar.gz") },
    "darwin-x86_64": { signature: "sig", url: url("Boba Job Find.app.tar.gz") },
    "windows-x86_64": { signature: "sig", url: url("Boba Job Find_0.3.0_x64-setup.exe") },
    "linux-x86_64": { signature: "sig", url: url("Boba Job Find_0.3.0_amd64.AppImage.tar.gz") },
  },
};

const draft = (assets = ASSETS) => ({ isDraft: true, assets: assets.map((name) => ({ name })) });
const published = (assets = ASSETS) => ({ ...draft(assets), isDraft: false });
const okProbe = () => vi.fn(async () => ({ ok: true, status: 200 }));

describe("verifyRelease (draft)", () => {
  it("accepts a complete draft without asking for publication", async () => {
    const probe = okProbe();
    const result = await verifyRelease({
      tag: TAG,
      repo: REPO,
      mode: "draft",
      release: draft(),
      latest: LATEST,
      probe,
    });
    expect(result).toEqual({ ok: true, errors: [] });
    expect(probe).not.toHaveBeenCalled();
  });

  it("names a missing installer", async () => {
    const result = await verifyRelease({
      tag: TAG,
      repo: REPO,
      mode: "draft",
      release: draft(ASSETS.filter((a) => !a.endsWith(".dmg"))),
      latest: LATEST,
      probe: okProbe(),
    });
    expect(result.ok).toBe(false);
    expect(result.errors).toContain("missing the macOS DMG");
  });

  it("rejects asset API ids that fix-latest-json.mjs should have rewritten", async () => {
    const result = await verifyRelease({
      tag: TAG,
      repo: REPO,
      mode: "draft",
      release: draft(),
      latest: {
        ...LATEST,
        platforms: {
          ...LATEST.platforms,
          "darwin-aarch64": { signature: "sig", url: "https://api.github.com/repos/o/r/x" },
        },
      },
      probe: okProbe(),
    });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes("not a public download URL"))).toBe(true);
  });
});

describe("verifyRelease (published)", () => {
  it("fails while the release is still a draft", async () => {
    const probe = okProbe();
    const result = await verifyRelease({
      tag: TAG,
      repo: REPO,
      mode: "published",
      release: draft(),
      latest: LATEST,
      probe,
    });
    expect(result.ok).toBe(false);
    expect(result.errors).toContain(`release ${TAG} is still a draft; publish it`);
    expect(probe).not.toHaveBeenCalled();
  });

  it("accepts a published release whose urls resolve", async () => {
    const probe = okProbe();
    const result = await verifyRelease({
      tag: TAG,
      repo: REPO,
      mode: "published",
      release: published(),
      latest: LATEST,
      probe,
    });
    expect(result).toEqual({ ok: true, errors: [] });
    expect(probe).toHaveBeenCalledTimes(4);
  });

  it("names every platform whose public url does not resolve", async () => {
    const probe = vi.fn(async (target) =>
      target === url("Boba Job Find.app.tar.gz")
        ? { ok: false, status: 404 }
        : { ok: true, status: 200 },
    );
    const result = await verifyRelease({
      tag: TAG,
      repo: REPO,
      mode: "published",
      release: published(),
      latest: LATEST,
      probe,
    });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes("darwin-aarch64") && e.includes("404"))).toBe(true);
    expect(result.errors.some((e) => e.includes("darwin-x86_64") && e.includes("404"))).toBe(true);
    expect(result.errors.some((e) => e.includes("windows-x86_64"))).toBe(false);
  });

  it("reports a probe that throws", async () => {
    const probe = vi.fn(async () => {
      throw new Error("network down");
    });
    const result = await verifyRelease({
      tag: TAG,
      repo: REPO,
      mode: "published",
      release: published(),
      latest: LATEST,
      probe,
    });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes("network down"))).toBe(true);
  });

  it("requires the repository to build public urls", async () => {
    const result = await verifyRelease({
      tag: TAG,
      repo: "",
      mode: "published",
      release: published(),
      latest: LATEST,
      probe: okProbe(),
    });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes("--repo"))).toBe(true);
  });
});

describe("verifyRelease (mode)", () => {
  it("rejects an unknown mode", async () => {
    const result = await verifyRelease({
      tag: TAG,
      repo: REPO,
      mode: "whatever",
      release: draft(),
      latest: LATEST,
      probe: okProbe(),
    });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes("--draft") && e.includes("--published"))).toBe(
      true,
    );
  });
});
