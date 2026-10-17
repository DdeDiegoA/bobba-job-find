// S10.5 AC1: the release tag, package.json, tauri.conf.json and the Cargo
// workspace must agree, or the workflow stops before building anything.
import { describe, expect, it } from "vitest";
import { cargoWorkspaceVersion, checkVersions, parseTag } from "./version.mjs";

describe("parseTag", () => {
  it("accepts a plain v-prefixed semver tag", () => {
    expect(parseTag("v1.2.3")).toEqual({ ok: true, version: "1.2.3" });
  });

  it("accepts a beta tag and keeps its pre-release part", () => {
    expect(parseTag("v0.2.0-beta.1")).toEqual({ ok: true, version: "0.2.0-beta.1" });
  });

  it("rejects a tag without the v prefix", () => {
    const result = parseTag("1.2.3");
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/v\d/);
  });

  it("rejects a branch-looking string", () => {
    expect(parseTag("main").ok).toBe(false);
  });
});

describe("checkVersions", () => {
  const ok = {
    tag: "v0.3.0",
    packageVersion: "0.3.0",
    tauriVersion: "0.3.0",
    cargoVersion: "0.3.0",
  };

  it("passes when every version matches the tag", () => {
    expect(checkVersions(ok)).toEqual({ ok: true, errors: [] });
  });

  it("names each file whose version drifted", () => {
    const result = checkVersions({ ...ok, packageVersion: "0.2.0", cargoVersion: "0.2.9" });
    expect(result.ok).toBe(false);
    expect(result.errors).toEqual([
      "package.json version 0.2.0 does not match tag v0.3.0",
      "Cargo.toml [workspace.package] version 0.2.9 does not match tag v0.3.0",
    ]);
  });

  it("reports a invalid tag instead of comparing it", () => {
    const result = checkVersions({ ...ok, tag: "0.3.0" });
    expect(result.ok).toBe(false);
    expect(result.errors[0]).toMatch(/tag/);
  });
});

describe("cargoWorkspaceVersion", () => {
  it("reads the workspace package version", () => {
    const toml = ['[workspace]', 'members = ["a"]', '', '[workspace.package]', 'version = "0.4.1"'].join(
      "\n",
    );
    expect(cargoWorkspaceVersion(toml)).toBe("0.4.1");
  });

  it("does not pick up a member crate's own version", () => {
    const toml = ['[package]', 'name = "x"', 'version = "9.9.9"', '', '[workspace.package]', 'version = "1.0.0"'].join(
      "\n",
    );
    expect(cargoWorkspaceVersion(toml)).toBe("1.0.0");
  });

  it("returns null when there is no workspace version", () => {
    expect(cargoWorkspaceVersion('[package]\nname = "x"')).toBeNull();
  });
});
