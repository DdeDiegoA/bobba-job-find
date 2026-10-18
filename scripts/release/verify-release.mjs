#!/usr/bin/env node
// Release gate after the builds (S10.5 AC1, AD-22 / FX21): every installer is
// present, `latest.json` resolves every platform and its version is the tag.
//
// Two modes, run at the two moments the publish job has to check:
//
//   node scripts/release/verify-release.mjs v1.2.3 release-assets --repo owner/repo --draft
//     Before publishing. Compares the platform urls with the release's asset
//     list only — a draft's assets are not downloadable yet, so nothing is
//     fetched and being a draft is expected.
//
//   node scripts/release/verify-release.mjs v1.2.3 release-assets --repo owner/repo --published
//     After publishing. Also fails while the release is a draft, and fetches
//     every platform url: an asset that does not resolve here is an update
//     that fails for installed apps.
//
// With --repo (default: GITHUB_REPOSITORY) every platform url must be the
// public download form for that repository and tag (REL5). The asset list is
// read from the release, never from the download directory.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { verifyRelease } from "./lib/verify-release.mjs";

const PROBE_ATTEMPTS = 3;
const PROBE_DELAY_MS = 4_000;

const positional = [];
const modes = [];
let repo = process.env.GITHUB_REPOSITORY ?? "";
const args = process.argv.slice(2);
for (let i = 0; i < args.length; i++) {
  const arg = args[i];
  if (arg === "--repo") {
    repo = args[++i] ?? "";
  } else if (arg === "--draft") {
    modes.push("draft");
  } else if (arg === "--published") {
    modes.push("published");
  } else {
    positional.push(arg);
  }
}
const [tag, assetsDir] = positional;

function fail(errors) {
  for (const error of errors) console.error(`::error::${error}`);
  process.exit(1);
}

if (!tag || !assetsDir) {
  fail([
    "usage: node scripts/release/verify-release.mjs <tag> <assets-dir> [--repo owner/repo] (--draft | --published)",
  ]);
}
if (modes.length !== 1) {
  fail(["pass exactly one of --draft (before publishing) or --published (after)"]);
}
const [mode] = modes;
if (!existsSync(assetsDir)) fail([`no assets directory at ${assetsDir}`]);

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

const latestPath = join(assetsDir, "latest.json");
let latest = null;
if (existsSync(latestPath)) {
  try {
    latest = JSON.parse(readFileSync(latestPath, "utf8"));
  } catch (error) {
    fail([`latest.json is not valid JSON: ${String(error.message ?? error)}`]);
  }
}

// A freshly published asset can answer 404 for a few seconds while the
// download endpoint catches up, so a miss is retried before it fails the run.
async function probe(url) {
  let last = { ok: false, status: "none" };
  for (let attempt = 1; attempt <= PROBE_ATTEMPTS; attempt++) {
    try {
      const response = await fetch(url, { method: "HEAD", redirect: "follow" });
      last = { ok: response.ok, status: response.status };
    } catch (error) {
      last = { ok: false, status: "none", error };
    }
    if (last.ok) return last;
    if (attempt < PROBE_ATTEMPTS) {
      await new Promise((resolve) => setTimeout(resolve, PROBE_DELAY_MS));
    }
  }
  return last;
}

const result = await verifyRelease({ tag, repo, mode, release, latest, probe });
if (!result.ok) fail(result.errors);
console.log(
  mode === "draft"
    ? `release draft ok: ${tag} has every installer and latest.json resolves all platforms`
    : `release published ok: ${tag} is not a draft and every update url resolves`,
);
