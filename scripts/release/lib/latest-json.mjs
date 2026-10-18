// REL5: the updater downloads the `latest.json` platform urls directly, so
// every one of them must be the public download URL of an attached asset.
// While the release is a draft tauri-action writes asset API URLs
// (`.../releases/assets/<id>`), which 404 once the release is published.
// These helpers rewrite them and verify the public form.

/** The URL the updater resolves for one platform asset on a published release. */
export function publicDownloadUrl({ repo, tag, name }) {
  const path = encodeURIComponent(tag);
  const file = encodeURIComponent(name);
  return `https://github.com/${repo}/releases/download/${path}/${file}`;
}

/** `{repo, tag, name}` when `url` is a GitHub public asset download URL. */
export function parseDownloadUrl(url) {
  const match = /^https:\/\/github\.com\/([^/]+\/[^/]+)\/releases\/download\/([^/]+)\/(.+)$/.exec(
    String(url),
  );
  if (!match) return null;
  try {
    return {
      repo: match[1],
      tag: decodeURIComponent(match[2]),
      name: decodeURIComponent(match[3]),
    };
  } catch {
    return null;
  }
}

/** The asset id when `url` is a GitHub asset API URL (or a bare id). */
export function assetIdOf(url) {
  const text = String(url);
  const api = /^https:\/\/api\.github\.com\/repos\/[^/]+\/[^/]+\/releases\/assets\/(\d+)\/?$/.exec(
    text,
  );
  if (api) return api[1];
  return /^\d+$/.test(text) ? text : null;
}

/**
 * `{json, errors}`: a copy of `latest.json` with every platform url rewritten
 * to `publicDownloadUrl`, and the reasons when one cannot be resolved.
 *
 * `assets` is the release's asset list as `[{id, name}]`. Signatures and every
 * other field are copied verbatim; the input object is never mutated.
 */
export function fixLatestJson({ json, tag, repo, assets }) {
  if (!json || typeof json !== "object") {
    return { json, errors: ["latest.json is missing"] };
  }
  if (!tag || !repo) {
    return { json, errors: ["a tag and an owner/repo are required to build download URLs"] };
  }

  const byId = new Map(assets.map((asset) => [String(asset.id), String(asset.name)]));
  const names = new Set(assets.map((asset) => String(asset.name)));
  const hasPlatforms =
    json.platforms && typeof json.platforms === "object" && Object.keys(json.platforms).length > 0;
  if (!hasPlatforms) {
    return { json, errors: ["latest.json has no platforms to fix"] };
  }
  const platforms = json.platforms;
  const errors = [];
  const fixedPlatforms = {};

  for (const [platform, entry] of Object.entries(platforms)) {
    const url = String(entry?.url ?? "");
    const id = assetIdOf(url);
    const download = parseDownloadUrl(url);
    let name = null;

    if (id) {
      name = byId.get(id) ?? null;
      if (!name) {
        errors.push(
          `latest.json ${platform} points at asset ${id}, which is not in release ${tag}`,
        );
      }
    } else if (download) {
      if (download.repo !== repo) {
        errors.push(`latest.json ${platform} points at ${download.repo}, not ${repo}`);
      } else if (download.tag !== tag) {
        errors.push(`latest.json ${platform} points at tag ${download.tag}, not ${tag}`);
      } else if (!names.has(download.name)) {
        errors.push(
          `latest.json ${platform} points at ${download.name}, which is not an asset of release ${tag}`,
        );
      } else {
        name = download.name;
      }
    } else {
      errors.push(`latest.json ${platform} has an invalid url`);
    }

    fixedPlatforms[platform] = name
      ? { ...entry, url: publicDownloadUrl({ repo, tag, name }) }
      : { ...entry };
  }

  return { json: { ...json, platforms: fixedPlatforms }, errors };
}
