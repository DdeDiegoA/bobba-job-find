#!/usr/bin/env node
// Release guard (S10.5 F5): the updater endpoint must match the repository
// that publishes the releases. In Actions the repository is GITHUB_REPOSITORY;
// locally pass `--repo owner/name`.
//
//   node scripts/release/verify-updater-endpoint.mjs
//   node scripts/release/verify-updater-endpoint.mjs --repo acme/boba-job-find
import { readFileSync } from "node:fs";
import { checkEndpoint } from "./lib/endpoint.mjs";

const flag = process.argv.indexOf("--repo");
const repository = flag === -1 ? process.env.GITHUB_REPOSITORY : process.argv[flag + 1];

if (!repository) {
  console.error("::error::no repository to compare against; pass --repo owner/name");
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
