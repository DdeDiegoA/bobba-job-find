// WiX/MSI version mapping for the tag-driven release (S10.5 AC1, AD-22).
//
// The app version stays semver (`1.0.0-beta.1`): the updater and the release
// guard read it. The MSI cannot carry a pre-release suffix — WiX wants
// `major.minor.patch.build`, all numeric, major/minor <= 255 and patch/build
// <= 65535 — so `bundle.windows.wix.version` gets its own value:
//
//   v1.0.0-beta.1   -> 1.0.0.1      (the beta counter becomes the build field)
//   v1.0.0-beta.12  -> 1.0.0.12
//   v1.0.0          -> 1.0.0.65535  (a stable release upgrades its own betas)
//
// release.yml writes this into `src-tauri/tauri.windows.conf.json`, which Tauri
// merges into tauri.conf.json on Windows only. The mapping is documented in
// docs/install/RELEASING.md.
import { parseTag } from "./version.mjs";

const MAX_FIELD = 255; // major, minor
const MAX_BUILD = 65535; // patch, build
// High enough that every beta of the same version sorts below its release.
const STABLE_BUILD = MAX_BUILD;

/** Last all-numeric identifier of a pre-release (`beta.12` -> 12), or 1. */
function prereleaseBuild(prerelease) {
  const numbers = prerelease.split(".").filter((part) => /^\d+$/.test(part));
  return numbers.length ? Number(numbers.at(-1)) : 1;
}

/** `{ok, version}` with the numeric WiX version for a release tag, or an error. */
export function wixVersion(tag) {
  const parsed = parseTag(tag);
  if (!parsed.ok) return { ok: false, error: parsed.error };

  const dash = parsed.version.indexOf("-");
  const base = dash === -1 ? parsed.version : parsed.version.slice(0, dash);
  const prerelease = dash === -1 ? "" : parsed.version.slice(dash + 1);
  const [major, minor, patch] = base.split(".").map(Number);

  if (major > MAX_FIELD || minor > MAX_FIELD) {
    return {
      ok: false,
      error: `tag ${tag} has a major/minor above ${MAX_FIELD}; the MSI version cannot carry it`,
    };
  }
  if (patch > MAX_BUILD) {
    return {
      ok: false,
      error: `tag ${tag} has a patch above ${MAX_BUILD}; the MSI version cannot carry it`,
    };
  }

  const build = prerelease === "" ? STABLE_BUILD : prereleaseBuild(prerelease);
  if (build > MAX_BUILD) {
    return {
      ok: false,
      error: `tag ${tag} has a pre-release number above ${MAX_BUILD}; the MSI version cannot carry it (renumber the beta)`,
    };
  }

  return { ok: true, version: `${major}.${minor}.${patch}.${build}` };
}

/** The platform config fragment Tauri merges on Windows (`tauri.windows.conf.json`). */
export function wixConfig(version) {
  return { bundle: { windows: { wix: { version } } } };
}
