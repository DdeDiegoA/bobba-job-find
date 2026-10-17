// What a published release must contain (S10.5 AC1, NFR-16, AD-22): one
// installer per OS, updater signatures, and a `latest.json` whose version is
// the tag and whose platform URLs point at assets that actually exist.

/** The platform keys the updater asks for; a universal DMG fills both macOS. */
export const PLATFORMS = ["darwin-aarch64", "darwin-x86_64", "windows-x86_64", "linux-x86_64"];

const INSTALLERS = [
  { test: (name) => name.endsWith(".dmg"), error: "missing the macOS DMG" },
  { test: (name) => name.endsWith(".msi"), error: "missing the Windows MSI" },
  { test: (name) => /-setup\.exe$/i.test(name), error: "missing the Windows NSIS installer" },
  { test: (name) => name.endsWith(".AppImage"), error: "missing the Linux AppImage" },
  { test: (name) => name.endsWith(".deb"), error: "missing the Linux .deb" },
  { test: (name) => name.endsWith(".rpm"), error: "missing the Linux .rpm" },
];

/** `{ok, errors}` for the names in a release's asset list. */
export function checkAssets(assetNames) {
  const names = assetNames.map(String);
  const errors = [];
  for (const installer of INSTALLERS) {
    if (!names.some(installer.test)) errors.push(installer.error);
  }
  // Every updater bundle is minisign-signed: macOS, Windows and Linux.
  const signatures = names.filter((name) => name.endsWith(".sig")).length;
  if (signatures < 3) {
    errors.push(`missing updater signature files (found ${signatures}, expected at least 3)`);
  }
  if (!names.includes("latest.json")) {
    errors.push("missing latest.json (the updater manifest)");
  }
  return { ok: errors.length === 0, errors };
}

/** The asset basename a manifest URL points at (URL-encoded names decoded). */
function urlBasename(url) {
  try {
    const path = new URL(String(url)).pathname;
    const last = path.split("/").pop() ?? "";
    return decodeURIComponent(last);
  } catch {
    return null;
  }
}

/** `{ok, errors}` for `latest.json` against the tag and the published assets. */
export function checkLatestJson({ json, tag, assets }) {
  if (!json) {
    return { ok: false, errors: ["latest.json is missing from the release"] };
  }
  const errors = [];
  const want = String(tag ?? "").replace(/^v/, "");
  if (String(json.version) !== want) {
    errors.push(`latest.json version ${json.version} does not match tag ${tag}`);
  }
  const platforms = json.platforms ?? {};
  for (const platform of PLATFORMS) {
    const entry = platforms[platform];
    if (!entry) {
      errors.push(`latest.json has no ${platform} entry`);
      continue;
    }
    if (!entry.signature) {
      errors.push(`latest.json ${platform} entry has no signature`);
    }
    const basename = urlBasename(entry.url);
    if (!basename) {
      errors.push(`latest.json ${platform} entry has an invalid url`);
    } else if (!assets.includes(basename)) {
      errors.push(`latest.json ${platform} points at ${basename}, which is not a published asset`);
    }
  }
  return { ok: errors.length === 0, errors };
}
