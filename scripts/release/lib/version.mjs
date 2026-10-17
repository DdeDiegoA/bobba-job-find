// Version agreement for the tag-driven release (AD-22, S10.5 AC1).
// Pure functions so the workflow's guard job is testable without a checkout of
// a particular version.

const TAG = /^v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)$/;

/** Parse a release tag. Returns `{ok, version}` or `{ok: false, error}`. */
export function parseTag(tag) {
  const match = TAG.exec(String(tag ?? "").trim());
  if (!match) {
    return {
      ok: false,
      error: `release tags look like v1.2.3 (or v1.2.3-beta.1); got "${tag ?? ""}"`,
    };
  }
  return { ok: true, version: match[1] };
}

/** The `[workspace.package] version` of a Cargo.toml, or null. */
export function cargoWorkspaceVersion(tomlText) {
  const lines = String(tomlText ?? "").split("\n");
  let inWorkspacePackage = false;
  for (const raw of lines) {
    const line = raw.trim();
    if (line.startsWith("[")) {
      inWorkspacePackage = line === "[workspace.package]";
      continue;
    }
    if (!inWorkspacePackage) continue;
    const match = /^version\s*=\s*"([^"]+)"/.exec(line);
    if (match) return match[1];
  }
  return null;
}

/**
 * Compare the tag with every version the build reads. One source of truth per
 * artifact: package.json (web), tauri.conf.json (bundle) and the Cargo
 * workspace (binary metadata).
 */
export function checkVersions({ tag, packageVersion, tauriVersion, cargoVersion }) {
  const parsed = parseTag(tag);
  if (!parsed.ok) return { ok: false, errors: [parsed.error] };

  const errors = [];
  const want = parsed.version;
  if (packageVersion !== want) {
    errors.push(`package.json version ${packageVersion} does not match tag ${tag}`);
  }
  if (tauriVersion !== want) {
    errors.push(`src-tauri/tauri.conf.json version ${tauriVersion} does not match tag ${tag}`);
  }
  if (cargoVersion !== want) {
    errors.push(`Cargo.toml [workspace.package] version ${cargoVersion} does not match tag ${tag}`);
  }
  return { ok: errors.length === 0, errors };
}
