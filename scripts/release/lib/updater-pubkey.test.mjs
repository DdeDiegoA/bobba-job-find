// S10.5 AC2 / AD-22: the updater pubkey in tauri.conf.json must be a real
// minisign public key, so releases are always verified and never silently
// unverified.
import { describe, expect, it } from "vitest";
import { checkPubkey, decodePubkey, PUBLIC_KEY_PREFIX, SECRET_KEY_MARKER } from "./updater-pubkey.mjs";

function b64(text) {
  return Buffer.from(text, "utf8").toString("base64");
}

const VALID = b64(
  "untrusted comment: minisign public key: C7B79E7FC4754EC6\nRWTGTnXEf563x71Cph2ZWcpR/kLB/lZTnB4OVHFHTBbfjFkdfPiVI5+s\n",
);

describe("decodePubkey", () => {
  it("decodes the base64-wrapped minisign public key file", () => {
    expect(decodePubkey(VALID)).toContain("untrusted comment: minisign public key");
  });

  it("returns null when the value is not base64", () => {
    expect(decodePubkey("not a key!")).toBeNull();
  });
});

describe("checkPubkey", () => {
  it("accepts a minisign public key", () => {
    expect(checkPubkey(VALID)).toEqual({ ok: true, errors: [] });
  });

  it("rejects an empty value (placeholder)", () => {
    const result = checkPubkey("");
    expect(result.ok).toBe(false);
    expect(result.errors[0]).toMatch(/minisign public key/);
  });

  it("rejects a base64 value that is not a minisign key file", () => {
    const result = checkPubkey(b64("hello world\n"));
    expect(result.ok).toBe(false);
    expect(result.errors[0]).toMatch(/minisign public key/);
  });

  it("rejects a secret key pasted by mistake", () => {
    const result = checkPubkey(
      b64(
        "untrusted comment: minisign encrypted secret key\nRWQAA...\n",
      ),
    );
    expect(result.ok).toBe(false);
    expect(result.errors[0]).toMatch(/secret key/i);
  });

  it("rejects a public key whose key line is missing", () => {
    const result = checkPubkey(b64("untrusted comment: minisign public key: C7B79E7FC4754EC6\n"));
    expect(result.ok).toBe(false);
    expect(result.errors[0]).toMatch(/key line/);
  });
});

describe("constants", () => {
  it("names the minisign public-key prefix the plugin expects", () => {
    expect(PUBLIC_KEY_PREFIX).toBe("untrusted comment: minisign public key");
  });
  it("has a distinct secret marker", () => {
    expect(SECRET_KEY_MARKER).toContain("secret key");
  });
});
