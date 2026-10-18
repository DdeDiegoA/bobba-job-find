#!/usr/bin/env node
// Release guard (S10.5 F5): the updater endpoint must match the repository
// that publishes the releases. The expected slug comes from the workflow's
// repository, or the checkout's `origin` locally — never from a constant, so
// the endpoint cannot drift from the repo it ships from (REL1).
//
//   node scripts/release/verify-updater-endpoint.mjs
//   node scripts/release/verify-updater-endpoint.mjs --repo acme/boba-job-find
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { checkEndpoint, repoFromRemote } from "./lib/endpoint.mjs";

function originRepository() {
  try {
    return repoFromRemote(
      execFileSync("git", ["remote", "get-url", "origin"], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      }),
    );
  } catch {
    return null;
  }
}

const flag = process.argv.indexOf("--repo");
const repository =
  flag === -1 ? process.env.GITHUB_REPOSITORY || originRepository() : process.argv[flag + 1];

if (!repository) {
  console.error(
    "::error::no repository to compare against; pass --repo owner/name, or run in a checkout whose origin is github.com/<owner>/<repo>",
  );
  process.exit(1);
}

const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
const endpoint = config.plugins?.updater?.endpoints?.[0] ?? "";
const result = checkEndpoint(endpoint, repository);

if (!result.ok) {
  for (const error of result.errors) console.error(`::error::${error}`);
  process.exit(1);
}
console.log(`updater endpoint ok: ${endpoint} -> ${repository}`);
