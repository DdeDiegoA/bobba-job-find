// S10.5 AC1 / FX21: the publish job verifies twice, because the truth changes
// when the release is published.
//
//   draft     — asset names and the public url form. A draft's assets are not
//               downloadable yet, so nothing is fetched and being a draft is
//               not an error: this pass runs BEFORE `gh release edit --draft=false`.
//   published — not a draft any more, and every platform url the updater will
//               fetch resolves (a 404 here means installed apps cannot update).
//
// The probe is injected so the parsing stays unit-testable without network.

import { parseDownloadUrl } from "./latest-json.mjs";
import { checkAssets, checkLatestJson, PLATFORMS } from "./release-manifest.mjs";

export const MODES = ["draft", "published"];

/** The release's asset names, from `gh release view --json assets`. */
function assetNames(release) {
  return (release?.assets ?? []).map((asset) => String(asset?.name ?? asset));
}

/**
 * `{ok, errors}` for one verification pass.
 *
 * `release` is `gh release view --json isDraft,assets`; `latest` is the parsed
 * `latest.json`; `probe(url)` resolves to `{ok, status}` (published only).
 */
export async function verifyRelease({ tag, repo, mode, release, latest, probe }) {
  if (!MODES.includes(mode)) {
    return {
      ok: false,
      errors: [`unknown mode "${mode}"; pass --draft or --published`],
    };
  }
  if (mode === "published" && !repo) {
    return {
      ok: false,
      errors: ["--repo owner/repo (or GITHUB_REPOSITORY) is required with --published"],
    };
  }

  const assets = assetNames(release);
  const errors = [
    ...checkAssets(assets).errors,
    ...checkLatestJson({ json: latest, tag, assets, repo: repo || undefined }).errors,
  ];

  if (mode === "draft") return { ok: errors.length === 0, errors };

  if (release?.isDraft !== false) {
    errors.push(`release ${tag} is still a draft; publish it`);
  }
  // Probe only once the urls themselves are known good, so a bad manifest
  // reports the manifest error instead of four fetch failures.
  if (errors.length === 0) {
    for (const platform of PLATFORMS) {
      const url = latest?.platforms?.[platform]?.url;
      if (!url) continue;
      const expected = parseDownloadUrl(url)?.name;
      let result;
      try {
        result = await probe(String(url));
      } catch (error) {
        errors.push(
          `latest.json ${platform} url did not resolve: ${String(error?.message ?? error)}`,
        );
        continue;
      }
      if (result?.ok !== true) {
        errors.push(
          `latest.json ${platform} url did not resolve (HTTP ${result?.status ?? "unknown"}): ${expected ?? url}`,
        );
      }
    }
  }
  return { ok: errors.length === 0, errors };
}
