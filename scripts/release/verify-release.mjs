#!/usr/bin/env node
// Release gate after the builds (S10.5 AC1, AD-22): every installer is present,
// `latest.json` resolves every platform, its version is the tag, and the
// release is published — never left as a draft.
//
//   gh release download v1.2.3 --dir release-assets
//   node scripts/release/verify-release.mjs v1.2.3 release-assets
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { checkAssets, checkLatestJson } from "./lib/release-manifest.mjs";

const [tag, assetsDir] = process.argv.slice(2);

function fail(errors) {
  for (const error of errors) console.error(`::error::${error}`);
  process.exit(1);
}

if (!tag || !assetsDir) {
  fail(["usage: node scripts/release/verify-release.mjs <tag> <assets-dir>"]);
}
if (!existsSync(assetsDir)) fail([`no assets directory at ${assetsDir}`]);

const assets = readdirSync(assetsDir, { recursive: true, withFileTypes: true })
  .filter((entry) => entry.isFile())
  .map((entry) => entry.name);

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
const manifestResult = checkLatestJson({ json: latest, tag, assets });
errors.push(...manifestResult.errors);

// A published release is what makes latest.json reachable at the fixed
// /releases/latest/download URL the app is configured with.
try {
  const view = execFileSync(
    "gh",
    ["release", "view", tag, "--json", "isDraft,isPrerelease,tagName", "--jq", "."],
    { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
  );
  const release = JSON.parse(view);
  if (release.isDraft) errors.push(`release ${tag} is still a draft; publish it`);
} catch (error) {
  errors.push(`cannot read release ${tag} from GitHub: ${String(error.stderr ?? error).trim()}`);
}

if (errors.length) fail(errors);
console.log(`release ok: ${tag} has every installer, latest.json resolves all platforms, not a draft`);
