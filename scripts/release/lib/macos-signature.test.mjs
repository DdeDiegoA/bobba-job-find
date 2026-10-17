// S10.5 AC3 / ADR-009: every macOS build carries the one persistent
// project-owned self-signed identity and its pinned designated requirement.
import { describe, expect, it } from "vitest";
import {
  checkMacSignature,
  normalizeSha1,
  parseCodesignDv,
  parseDesignatedRequirement,
  parsePin,
} from "./macos-signature.mjs";

const PIN = "0123456789abcdef0123456789abcdef01234567";

const DV_OK = `Executable=/tmp/Boba Job Find.app/Contents/MacOS/boba-job-find
Identifier=app.bobajobfind
Format=app bundle with Mach-O universal (x86_64 arm64)
CodeDirectory v=20400 size=1600 flags=0x0(none) hashes=52+3 location=embedded
Signature size=4800
Authority=Boba Job Find Self-Signed
Authority=Boba Job Find Self-Signed
Signed Time=Oct 17, 2026 at 4:00:00 PM
Info.plist entries=22
TeamIdentifier=not set
Sealed Resources version=2 rules=13 files=41
`;

const DV_ADHOC = `Executable=/tmp/Boba Job Find.app/Contents/MacOS/boba-job-find
Identifier=app.bobajobfind
CodeDirectory v=20400 size=1600 flags=0x2(adhoc) hashes=52+3 location=embedded
Signature=adhoc
Info.plist entries=22
TeamIdentifier=not set
`;

// A self-signed identity anchors on `certificate root` (codesign's own output);
// a CA-issued one anchors on `certificate leaf`. Both are accepted.
const DR_OK = `designated => identifier "app.bobajobfind" and certificate root = H"${PIN}"\n`;
const DR_LEAF = `designated => identifier "app.bobajobfind" and certificate leaf = H"${PIN}"\n`;
const DR_WRONG = `designated => identifier "app.bobajobfind" and certificate root = H"fedcba9876543210fedcba9876543210fedcba98"\n`;
const DR_ADHOC = 'designated => cdhash H"1111111111111111111111111111111111111111"\n';

describe("parseCodesignDv", () => {
  it("reads the identifier and the signature kind", () => {
    const dv = parseCodesignDv(DV_OK);
    expect(dv.identifier).toBe("app.bobajobfind");
    expect(dv.signature).toBeUndefined();
    expect(dv.authorities).toEqual(["Boba Job Find Self-Signed", "Boba Job Find Self-Signed"]);
  });

  it("flags an ad-hoc signature", () => {
    expect(parseCodesignDv(DV_ADHOC).signature).toBe("adhoc");
  });
});

describe("parseDesignatedRequirement", () => {
  it("reads the identifier and the self-signed root hash", () => {
    expect(parseDesignatedRequirement(DR_OK)).toEqual({
      identifier: "app.bobajobfind",
      anchor: "root",
      certSha1: PIN,
    });
  });

  it("also reads a CA-issued leaf requirement", () => {
    expect(parseDesignatedRequirement(DR_LEAF).anchor).toBe("leaf");
  });

  it("returns null for an ad-hoc requirement", () => {
    expect(parseDesignatedRequirement(DR_ADHOC)).toBeNull();
  });
});

describe("normalizeSha1", () => {
  it("lowercases, trims and removes separators", () => {
    expect(normalizeSha1(" 01:23:AB cd EF ")).toBe("0123abcdef");
  });
});

describe("checkMacSignature", () => {
  it("passes a build signed with the pinned identity", () => {
    expect(checkMacSignature({ dv: DV_OK, dr: DR_OK, pinnedSha1: PIN })).toEqual({
      ok: true,
      errors: [],
    });
  });

  it("fails an ad-hoc build with the ADR-009 message", () => {
    const result = checkMacSignature({ dv: DV_ADHOC, dr: DR_ADHOC, pinnedSha1: PIN });
    expect(result.ok).toBe(false);
    expect(result.errors).toContain(
      "the build is ad-hoc signed; ADR-009 requires the persistent project identity",
    );
    expect(result.errors.some((e) => e.includes("designated requirement"))).toBe(true);
  });

  it("fails when the certificate leaf differs from the pin", () => {
    const result = checkMacSignature({ dv: DV_OK, dr: DR_WRONG, pinnedSha1: PIN });
    expect(result.ok).toBe(false);
    expect(result.errors).toContain(
      `designated requirement root fedcba9876543210fedcba9876543210fedcba98 does not match pinned ${PIN}`,
    );
  });

  it("fails when the identifier is not the app's", () => {
    const result = checkMacSignature({
      dv: DV_OK.replace("Identifier=app.bobajobfind", "Identifier=com.example.other"),
      dr: DR_OK,
      pinnedSha1: PIN,
    });
    expect(result.ok).toBe(false);
    expect(result.errors).toContain("codesign reports identifier com.example.other, expected app.bobajobfind");
  });

  it("fails closed when no pin is configured", () => {
    const result = checkMacSignature({ dv: DV_OK, dr: DR_OK, pinnedSha1: "" });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes("no pinned certificate"))).toBe(true);
  });
});

describe("parsePin", () => {
  it("reads the assignment and ignores comments", () => {
    const text = `# pinned\nBOBA_MACOS_CERT_SHA1=${PIN}\n`;
    expect(parsePin(text)).toBe(PIN);
  });

  it("returns null for an empty or malformed pin", () => {
    expect(parsePin("BOBA_MACOS_CERT_SHA1=\n")).toBeNull();
    expect(parsePin("BOBA_MACOS_CERT_SHA1=nothex\n")).toBeNull();
  });
});
