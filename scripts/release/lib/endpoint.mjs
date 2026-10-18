// The updater endpoint must name the repository that actually publishes the
// releases (S10.5 review F5). `latest.json` is resolved from that URL by every
// installed app, so a stale slug is a silent, permanent 404.

const GITHUB_RELEASES =
  /^https:\/\/github\.com\/([^/\s]+)\/([^/\s]+)\/releases\/latest\/download\/latest\.json$/;

/** `owner/repo` of a GitHub latest-release endpoint, or null. */
export function endpointRepo(url) {
  const match = GITHUB_RELEASES.exec(String(url ?? "").trim());
  return match ? `${match[1]}/${match[2]}` : null;
}

// The repository the checkout belongs to. Parsed from a git remote URL so the
// guard compares the endpoint with the origin instead of a hard-coded slug,
// and the two can never drift apart again (REL1).
const REMOTE =
  /^(?:git@github\.com:|(?:ssh|https):\/\/(?:git@)?github\.com\/)([^/\s]+)\/([^/\s]+)$/;

/** `owner/repo` of a GitHub remote URL (ssh, ssh:// or https), or null. */
export function repoFromRemote(remote) {
  const text = String(remote ?? "").trim().replace(/\.git$/, "");
  const match = REMOTE.exec(text);
  return match ? `${match[1]}/${match[2]}` : null;
}

/** `{ok, errors}` for the endpoint against the publishing repository. */
export function checkEndpoint(url, repository) {
  const repo = endpointRepo(url);
  if (!repo) {
    return {
      ok: false,
      errors: [
        "plugins.updater.endpoints[0] is not an https://github.com/<owner>/<repo>/releases/latest/download/latest.json URL",
      ],
    };
  }
  if (repo.toLowerCase() !== String(repository ?? "").trim().toLowerCase()) {
    return {
      ok: false,
      errors: [
        `the updater endpoint points at ${repo}, but this repository is ${repository}; update src-tauri/tauri.conf.json before tagging a release`,
      ],
    };
  }
  return { ok: true, errors: [] };
}
