#!/usr/bin/env node
// Release guard (S10.5 AC1, AD-22): the tag, package.json, tauri.conf.json and
// the Cargo workspace must carry exactly the same version before anything is
// built. Usage: node scripts/release/check-version.mjs [v1.2.3]
import { readFileSync } from "node:fs";
import { cargoWorkspaceVersion, checkVersions } from "./lib/version.mjs";

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

const tag = process.argv[2] ?? process.env.GITHUB_REF_NAME ?? "";
const result = checkVersions({
  tag,
  packageVersion: readJson("package.json").version,
  tauriVersion: readJson("src-tauri/tauri.conf.json").version,
  cargoVersion: cargoWorkspaceVersion(readFileSync("Cargo.toml", "utf8")),
});

if (!result.ok) {
  for (const error of result.errors) console.error(`::error::${error}`);
  process.exit(1);
}
console.log(`version ok: ${tag} matches package.json, tauri.conf.json and Cargo.toml`);
