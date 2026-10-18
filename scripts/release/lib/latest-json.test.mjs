// REL5: latest.json's platform urls must be public download URLs. While the
// release is a draft, tauri-action writes asset API URLs
// (`.../releases/assets/<id>`) that 404 for the updater once published.
import { describe, expect, it } from "vitest";
import { assetIdOf, fixLatestJson, parseDownloadUrl, publicDownloadUrl } from "./latest-json.mjs";

const TAG = "v1.0.0-beta.1";
const REPO = "DdeDiegoA/bobba-job-find";

const ASSETS = [
  { id: "652456087", name: "Boba.Job.Find_1.0.0-beta.1_amd64.AppImage" },
  { id: "652462513", name: "Boba.Job.Find_1.0.0-beta.1_x64-setup.exe" },
  { id: "652462424", name: "Boba.Job.Find_1.0.0-beta.1_x64_en-US.msi" },
  { id: "652477145", name: "Boba.Job.Find_1.0.0-beta.1_universal.app.tar.gz" },
  { id: "652477062", name: "Boba.Job.Find_1.0.0-beta.1_universal.dmg" },
  { id: "652477353", name: "latest.json" },
];

const SIG = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkK";

function draftManifest() {
  return {
    version: "1.0.0-beta.1",
    notes: "Beta 1",
    pub_date: "2026-10-18T04:33:47.803Z",
    platforms: {
      "darwin-aarch64": {
        signature: SIG,
        url: "https://api.github.com/repos/DdeDiegoA/bobba-job-find/releases/assets/652477145",
      },
      "darwin-aarch64-app": {
        signature: SIG,
        url: "https://api.github.com/repos/DdeDiegoA/bobba-job-find/releases/assets/652477145",
      },
      "windows-x86_64": {
        signature: SIG,
        url: "https://api.github.com/repos/DdeDiegoA/bobba-job-find/releases/assets/652462513",
      },
      "linux-x86_64": {
        signature: SIG,
        url: "https://api.github.com/repos/DdeDiegoA/bobba-job-find/releases/assets/652456087",
      },
    },
  };
}

describe("publicDownloadUrl", () => {
  it("builds the URL the updater downloads after publish", () => {
    expect(publicDownloadUrl({ repo: REPO, tag: TAG, name: "Boba Job Find.app.tar.gz" })).toBe(
      "https://github.com/DdeDiegoA/bobba-job-find/releases/download/v1.0.0-beta.1/Boba%20Job%20Find.app.tar.gz",
    );
  });
});

describe("parseDownloadUrl", () => {
  it("round-trips a public download URL, decoding the name", () => {
    const url = publicDownloadUrl({ repo: REPO, tag: TAG, name: "Boba Job Find.app.tar.gz" });
    expect(parseDownloadUrl(url)).toEqual({
      repo: REPO,
      tag: TAG,
      name: "Boba Job Find.app.tar.gz",
    });
  });

  it("rejects anything that is not a GitHub download URL", () => {
    expect(parseDownloadUrl("https://api.github.com/repos/a/b/releases/assets/1")).toBeNull();
    expect(parseDownloadUrl("https://example.com/x")).toBeNull();
  });
});

describe("assetIdOf", () => {
  it("reads the id from an asset API URL", () => {
    expect(
      assetIdOf("https://api.github.com/repos/DdeDiegoA/bobba-job-find/releases/assets/652456087"),
    ).toBe("652456087");
  });

  it("accepts a bare asset id", () => {
    expect(assetIdOf("652456087")).toBe("652456087");
  });

  it("rejects a public download URL", () => {
    expect(assetIdOf(publicDownloadUrl({ repo: REPO, tag: TAG, name: "x.AppImage" }))).toBeNull();
  });
});

describe("fixLatestJson", () => {
  it("rewrites every platform url to the public download URL of its asset", () => {
    const { json, errors } = fixLatestJson({
      json: draftManifest(),
      tag: TAG,
      repo: REPO,
      assets: ASSETS,
    });
    expect(errors).toEqual([]);
    expect(json.platforms["linux-x86_64"].url).toBe(
      "https://github.com/DdeDiegoA/bobba-job-find/releases/download/v1.0.0-beta.1/Boba.Job.Find_1.0.0-beta.1_amd64.AppImage",
    );
    expect(json.platforms["windows-x86_64"].url).toBe(
      "https://github.com/DdeDiegoA/bobba-job-find/releases/download/v1.0.0-beta.1/Boba.Job.Find_1.0.0-beta.1_x64-setup.exe",
    );
    // Both macOS keys point at the universal app bundle, and unknown keys are
    // fixed too, not only the four PLATFORMS the updater resolves.
    for (const platform of ["darwin-aarch64", "darwin-aarch64-app"]) {
      expect(json.platforms[platform].url).toBe(
        "https://github.com/DdeDiegoA/bobba-job-find/releases/download/v1.0.0-beta.1/Boba.Job.Find_1.0.0-beta.1_universal.app.tar.gz",
      );
    }
  });

  it("keeps every signature and every other field untouched", () => {
    const input = draftManifest();
    const before = JSON.parse(JSON.stringify(input));
    const { json } = fixLatestJson({ json: input, tag: TAG, repo: REPO, assets: ASSETS });
    expect(json.version).toBe(before.version);
    expect(json.notes).toBe(before.notes);
    expect(json.pub_date).toBe(before.pub_date);
    for (const platform of Object.keys(before.platforms)) {
      expect(json.platforms[platform].signature).toBe(before.platforms[platform].signature);
    }
    // Pure: the input is not mutated.
    expect(input).toEqual(before);
  });

  it("is idempotent on an already-public manifest", () => {
    const first = fixLatestJson({ json: draftManifest(), tag: TAG, repo: REPO, assets: ASSETS });
    const second = fixLatestJson({ json: first.json, tag: TAG, repo: REPO, assets: ASSETS });
    expect(second.errors).toEqual([]);
    expect(second.json).toEqual(first.json);
  });

  it("fails when an asset id is not in the release", () => {
    const json = draftManifest();
    json.platforms["linux-x86_64"].url =
      "https://api.github.com/repos/DdeDiegoA/bobba-job-find/releases/assets/999999999";
    const { errors } = fixLatestJson({ json, tag: TAG, repo: REPO, assets: ASSETS });
    expect(errors).toContain(
      `latest.json linux-x86_64 points at asset 999999999, which is not in release ${TAG}`,
    );
  });

  it("fails when a public url names another tag or repository", () => {
    const json = draftManifest();
    json.platforms["linux-x86_64"].url =
      "https://github.com/DdeDiegoA/bobba-job-find/releases/download/v1.0.0-beta.0/Boba.Job.Find_1.0.0-beta.1_amd64.AppImage";
    const wrongTag = fixLatestJson({ json, tag: TAG, repo: REPO, assets: ASSETS });
    expect(wrongTag.errors.some((e) => e.includes("tag v1.0.0-beta.0"))).toBe(true);

    json.platforms["linux-x86_64"].url =
      "https://github.com/someone/fork/releases/download/v1.0.0-beta.1/Boba.Job.Find_1.0.0-beta.1_amd64.AppImage";
    const wrongRepo = fixLatestJson({ json, tag: TAG, repo: REPO, assets: ASSETS });
    expect(wrongRepo.errors.some((e) => e.includes("someone/fork"))).toBe(true);
  });

  it("fails when an entry has no usable url", () => {
    const json = draftManifest();
    json.platforms["linux-x86_64"].url = "https://example.com/latest";
    expect(
      fixLatestJson({ json, tag: TAG, repo: REPO, assets: ASSETS }).errors,
    ).toContain("latest.json linux-x86_64 has an invalid url");
  });

  it("fails when the manifest has no platforms", () => {
    const result = fixLatestJson({
      json: { version: "1.0.0-beta.1" },
      tag: TAG,
      repo: REPO,
      assets: ASSETS,
    });
    expect(result.errors).toContain("latest.json has no platforms to fix");
  });
});
