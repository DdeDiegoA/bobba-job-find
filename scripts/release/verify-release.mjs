#!/usr/bin/env node
// Release gate after the builds (S10.5 AC1, AD-22): every installer is present,
// `latest.json` resolves every platform, its version is the tag, and the
// release is published — never left as a draft.
//
//   gh release download v1.2.3 --dir release-assets
//   node scripts/release/verify-release.mjs v1.2.3 release-assets --repo owner/repo
//
// With --repo (default: GITHUB_REPOSITORY) every platform url must already be
// the public download URL for that repository and tag (REL5). The assets of a
// draft are not publicly downloadable yet, so the check compares the url form
// and the asset list, never fetches the files.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { checkAssets, checkLatestJson } from "./lib/release-manifest.mjs";

const positional = [];
let repo = process.env.GITHUB_REPOSITORY ?? "";
const args = process.argv.slice(2);
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--repo") {
    repo = args[++i] ?? "";
  } else {
    positional.push(args[i]);
  }
}
const [tag, assetsDir] = positional;

function fail(errors) {
  for (const error of errors) console.error(`::error::${error}`);
  process.exit(1);
}

if (!tag || !assetsDir) {
  fail([
    "usage: node scripts/release/verify-release.mjs <tag> <assets-dir> [--repo owner/repo]",
  ]);
}
if (!existsSync(assetsDir)) fail([`no assets directory at ${assetsDir}`]);

// The asset list the check runs on is the release's, not the download
// directory's (REL5): a file missing from the release must fail even if a
// matching local copy exists.
let release;
try {
  const repoArgs = repo ? ["--repo", repo] : [];
  const view = execFileSync(
    "gh",
    [
      "release",
      "view",
      tag,
      ...repoArgs,
      "--json",
      "isDraft,isPrerelease,tagName,assets",
      "--jq",
      ".",
    ],
    { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
  );
  release = JSON.parse(view);
} catch (error) {
  fail([`cannot read release ${tag} from GitHub: ${String(error.stderr ?? error).trim()}`]);
}
const assets = (release.assets ?? []).map((asset) => String(asset.name));

const latestPath = join(assetsDir, "latest.json");
let latest = null;
if (existsSync(latestPath)) {
  try {
    latest = JSON.parse(readFileSync(latestPath, "utf8"));
  } catch (error) {
    fail([`latest.json is not valid JSON: ${String(error.message ?? error)}`]);
  }
}

const errors = [];
const assetsResult = checkAssets(assets);
errors.push(...assetsResult.errors);
const manifestResult = checkLatestJson({ json: latest, tag, assets, repo: repo || undefined });
errors.push(...manifestResult.errors);

// A published release is what makes latest.json reachable at the fixed
// /releases/latest/download URL the app is configured with.
if (release.isDraft) errors.push(`release ${tag} is still a draft; publish it`);

if (errors.length) fail(errors);
console.log(`release ok: ${tag} has every installer, latest.json resolves all platforms, not a draft`);
