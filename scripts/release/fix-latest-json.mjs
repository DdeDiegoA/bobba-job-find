#!/usr/bin/env node
// REL5: tauri-action writes draft asset API ids into latest.json's platform
// urls while the release is unpublished
// (`https://api.github.com/repos/<owner>/<repo>/releases/assets/<id>`). The
// updater downloads those urls directly after publish, so they must be the
// public download URL of the matching asset. This rewrites them in place:
// signatures and every other field are left untouched, and a missing asset is
// an error, never a guess.
//
//   node scripts/release/fix-latest-json.mjs v1.2.3 release-assets/latest.json \
//     --repo owner/repo
//
// --repo defaults to GITHUB_REPOSITORY. The asset list is read with `gh`, so a
// draft (which the REST tags endpoint does not serve) resolves fine.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { fixLatestJson } from "./lib/latest-json.mjs";

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

const [tag, latestPath] = positional;

function fail(errors) {
  for (const error of errors) console.error(`::error::${error}`);
  process.exit(1);
}

if (!tag || !latestPath) {
  fail([
    "usage: node scripts/release/fix-latest-json.mjs <tag> <latest.json> [--repo owner/repo]",
  ]);
}
if (!repo) fail(["--repo owner/repo (or GITHUB_REPOSITORY) is required"]);
if (!existsSync(latestPath)) fail([`no latest.json at ${latestPath}`]);

let json;
try {
  json = JSON.parse(readFileSync(latestPath, "utf8"));
} catch (error) {
  fail([`latest.json is not valid JSON: ${String(error.message ?? error)}`]);
}

// Drafts are excluded from `GET /releases/tags/<tag>`, so ask GraphQL through
// `gh release view`. The numeric asset id lives in `apiUrl`.
let assets;
try {
  const out = execFileSync(
    "gh",
    [
      "release",
      "view",
      tag,
      "--repo",
      repo,
      "--json",
      "assets",
      "--jq",
      '.assets | map({id: (.apiUrl | split("/") | last), name})',
    ],
    { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
  );
  assets = JSON.parse(out);
} catch (error) {
  fail([
    `cannot read the assets of release ${tag} from ${repo}: ${String(error.stderr ?? error).trim()}`,
  ]);
}

const fixed = fixLatestJson({ json, tag, repo, assets });
if (fixed.errors.length) fail(fixed.errors);

writeFileSync(latestPath, `${JSON.stringify(fixed.json, null, 2)}\n`);
console.log(
  `latest.json fixed: ${Object.keys(fixed.json.platforms).length} platform urls now point at public downloads for ${tag}`,
);
