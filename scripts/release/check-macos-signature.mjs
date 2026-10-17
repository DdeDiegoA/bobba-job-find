#!/usr/bin/env node
// macOS signing gate (S10.5 AC3, ADR-009). Run it on every .app the release
// pipeline produces, before the release is published:
//
//   node scripts/release/check-macos-signature.mjs "path/to/Boba Job Find.app"
//
// It fails when codesign reports an ad-hoc signature, when the bundle
// identifier is not `app.bobajobfind`, or when the designated requirement's
// certificate leaf differs from scripts/release/macos-identity.pin.
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { checkMacSignature, parsePin } from "./lib/macos-signature.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const app = args.find((arg) => !arg.startsWith("--"));
const pinPath = join(here, "macos-identity.pin");

function fail(errors) {
  for (const error of errors) console.error(`::error::${error}`);
  process.exit(1);
}

// `codesign -dv` and `-d -r-` write to stderr and exit non-zero on an unsigned
// bundle; capture both streams without throwing.
function codesign(args) {
  const run = spawnSync("codesign", args, { encoding: "utf8" });
  return `${run.stdout ?? ""}\n${run.stderr ?? ""}`;
}

if (!app) fail(["usage: node scripts/release/check-macos-signature.mjs <path/to/App.app>"]);
if (process.platform !== "darwin") fail(["the macOS signature check only runs on macOS"]);
if (!existsSync(app)) fail([`no app bundle at ${app}`]);

const dv = codesign(["-dv", "--verbose=4", app]);
const dr = codesign(["-d", "-r-", app]);
const verify = spawnSync("codesign", ["--verify", "--strict", app], { encoding: "utf8" });
if (verify.status !== 0) {
  fail([`codesign --verify --strict failed: ${`${verify.stdout ?? ""}${verify.stderr ?? ""}`.trim()}`]);
}

const pinText = existsSync(pinPath) ? readFileSync(pinPath, "utf8") : "";
const result = checkMacSignature({ dv, dr, pinnedSha1: parsePin(pinText) ?? "" });
if (!result.ok) fail(result.errors);

console.log(`macOS signature ok: ${app}`);
console.log(dv.trim());
console.log(dr.trim());
