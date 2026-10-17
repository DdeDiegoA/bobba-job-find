// S10.5 review F5: the updater endpoint must point at the repository that
// publishes the releases, or installed apps poll a 404 forever.
import { describe, expect, it } from "vitest";
import { checkEndpoint, endpointRepo } from "./endpoint.mjs";

const URL_OK = "https://github.com/acme/boba-job-find/releases/latest/download/latest.json";

describe("endpointRepo", () => {
  it("reads owner/repo from a GitHub releases URL", () => {
    expect(endpointRepo(URL_OK)).toBe("acme/boba-job-find");
  });

  it("rejects a non-GitHub host", () => {
    expect(endpointRepo("https://example.com/acme/boba-job-find/releases/latest")).toBeNull();
  });

  it("rejects a http URL", () => {
    expect(endpointRepo("http://github.com/acme/boba-job-find/releases/latest")).toBeNull();
  });
});

describe("checkEndpoint", () => {
  it("passes when the endpoint matches the publishing repository", () => {
    expect(checkEndpoint(URL_OK, "acme/boba-job-find")).toEqual({ ok: true, errors: [] });
  });

  it("is case-insensitive on the repository name", () => {
    expect(checkEndpoint(URL_OK, "ACME/Boba-Job-Find").ok).toBe(true);
  });

  it("fails when the endpoint names another repository", () => {
    const result = checkEndpoint(URL_OK, "diegoarenas/boba-job-find");
    expect(result.ok).toBe(false);
    expect(result.errors[0]).toContain("acme/boba-job-find");
    expect(result.errors[0]).toContain("diegoarenas/boba-job-find");
  });

  it("fails on a missing or malformed endpoint", () => {
    expect(checkEndpoint("", "acme/boba-job-find").ok).toBe(false);
    expect(checkEndpoint("not a url", "acme/boba-job-find").ok).toBe(false);
  });
});
