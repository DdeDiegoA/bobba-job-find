// The macOS release gate (S10.5 AC3, ADR-009): the app must be signed with the
// one persistent project-owned self-signed identity, its designated requirement
// must name `app.bobajobfind` and the pinned certificate leaf, and it must never
// be ad-hoc. Parsing is pure so the workflow can test it against recorded
// `codesign` output.

export const APP_IDENTIFIER = "app.bobajobfind";

/** Lowercase, stripped `codesign` hash (spaces and colons are cosmetic). */
export function normalizeSha1(value) {
  return String(value ?? "")
    .replace(/[\s:]/g, "")
    .toLowerCase();
}

/** Parse `codesign -dv --verbose=4` output (which goes to stderr). */
export function parseCodesignDv(text) {
  const dv = { authorities: [] };
  for (const raw of String(text ?? "").split("\n")) {
    const line = raw.trim();
    const identifier = /^Identifier=(.+)$/.exec(line);
    if (identifier) {
      dv.identifier = identifier[1].trim();
      continue;
    }
    const signature = /^Signature=(.+)$/.exec(line);
    if (signature) {
      // The only `Signature=` line is the ad-hoc marker; a real signature is
      // reported as `Signature size=…`.
      dv.signature = signature[1].trim();
      continue;
    }
    const team = /^TeamIdentifier=(.+)$/.exec(line);
    if (team) {
      dv.teamIdentifier = team[1].trim();
      continue;
    }
    const authority = /^Authority=(.+)$/.exec(line);
    if (authority) dv.authorities.push(authority[1].trim());
  }
  return dv;
}

/**
 * Parse `codesign -d -r- <app>` (the designated requirement, on stderr).
 * A self-signed identity anchors the requirement on `certificate root`; a
 * certificate issued by a CA anchors on `certificate leaf`. Both name exactly
 * one certificate hash, which is what the pin compares.
 * Returns null when the requirement is a bare cdhash (an ad-hoc build).
 */
export function parseDesignatedRequirement(text) {
  const line = String(text ?? "")
    .split("\n")
    .map((l) => l.trim())
    .find((l) => l.startsWith("designated =>"));
  if (!line) return null;
  const identifier = /identifier "([^"]+)"/.exec(line);
  const cert = /certificate (leaf|root) = H"([0-9A-Fa-f ]+)"/.exec(line);
  if (!identifier || !cert) return null;
  return { identifier: identifier[1], anchor: cert[1], certSha1: normalizeSha1(cert[2]) };
}

/** Read the committed pin file; null when absent, empty or malformed. */
export function parsePin(text) {
  const match = /^BOBA_MACOS_CERT_SHA1=([0-9a-fA-F]{40})\s*$/m.exec(String(text ?? ""));
  return match ? match[1].toLowerCase() : null;
}

/**
 * The release gate. Returns `{ok, errors}`; every error is a sentence a human
 * can act on, and a missing pin fails closed.
 */
export function checkMacSignature({ dv, dr, pinnedSha1 }) {
  const errors = [];
  const parsedDv = parseCodesignDv(dv);
  const parsedDr = parseDesignatedRequirement(dr);
  const pin = normalizeSha1(pinnedSha1);

  if (parsedDv.signature === "adhoc") {
    errors.push("the build is ad-hoc signed; ADR-009 requires the persistent project identity");
  }
  if (parsedDv.identifier !== APP_IDENTIFIER) {
    errors.push(
      `codesign reports identifier ${parsedDv.identifier ?? "none"}, expected ${APP_IDENTIFIER}`,
    );
  }

  if (!/^[0-9a-f]{40}$/.test(pin)) {
    errors.push(
      "no pinned certificate in scripts/release/macos-identity.pin; run scripts/release/generate-macos-identity.sh and commit the pin",
    );
  }

  if (!parsedDr) {
    errors.push(
      'the designated requirement is not certificate-based (expected identifier "app.bobajobfind" and certificate leaf/root = H"…"); ad-hoc builds change it on every update',
    );
  } else {
    if (parsedDr.identifier !== APP_IDENTIFIER) {
      errors.push(
        `the designated requirement names ${parsedDr.identifier}, expected ${APP_IDENTIFIER}`,
      );
    }
    if (/^[0-9a-f]{40}$/.test(pin) && parsedDr.certSha1 !== pin) {
      errors.push(
        `designated requirement ${parsedDr.anchor} ${parsedDr.certSha1} does not match pinned ${pin}`,
      );
    }
  }

  return { ok: errors.length === 0, errors };
}
