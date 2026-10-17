// Tauri's updater pubkey is the base64 of a minisign public key file:
//
//   untrusted comment: minisign public key: <KEYNUM>
//   RW…<base64 key>
//
// A placeholder or a secret key pasted by mistake would make the updater
// unverifiable (or, worse, let anyone sign updates), so the release guard
// refuses both before a build starts (S10.5 AC2, AD-22).

export const PUBLIC_KEY_PREFIX = "untrusted comment: minisign public key";
export const SECRET_KEY_MARKER = "minisign encrypted secret key";

/** Decode the base64 wrapper, or null when it is not base64. */
export function decodePubkey(value) {
  const text = String(value ?? "").replace(/\s+/g, "");
  if (!text || !/^[A-Za-z0-9+/]+={0,2}$/.test(text) || text.length % 4 !== 0) return null;
  return Buffer.from(text, "base64").toString("utf8");
}

/** `{ok, errors}` for a candidate tauri.conf.json `plugins.updater.pubkey`. */
export function checkPubkey(value) {
  const decoded = decodePubkey(value);
  if (decoded?.includes(SECRET_KEY_MARKER)) {
    return {
      ok: false,
      errors: ["plugins.updater.pubkey contains a secret key; only the public key may be committed"],
    };
  }
  if (!decoded || !decoded.includes(PUBLIC_KEY_PREFIX)) {
    return {
      ok: false,
      errors: [
        "plugins.updater.pubkey is not a minisign public key; run scripts/release/generate-updater-keys.sh and paste the printed key",
      ],
    };
  }
  const keyLine = decoded
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line.startsWith("RW"));
  if (!keyLine) {
    return { ok: false, errors: ["plugins.updater.pubkey has no minisign key line (it should start with RW)"] };
  }
  return { ok: true, errors: [] };
}
