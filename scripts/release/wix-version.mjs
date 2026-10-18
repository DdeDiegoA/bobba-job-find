#!/usr/bin/env node
// The numeric MSI version for a release tag (S10.5 AC1, AD-22, REL2).
//
//   node scripts/release/wix-version.mjs v1.0.0-beta.1
//   -> 1.0.0.1
//
// With --config it also writes the platform config Tauri merges on Windows, so
// the Windows build needs no hand edit per beta:
//
//   node scripts/release/wix-version.mjs v1.0.0-beta.1 \
//     --config src-tauri/tauri.windows.conf.json
//
// The mapping (and the limits WiX imposes) are documented in
// docs/install/RELEASING.md.
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { parseArgs } from "node:util";
import { wixConfig, wixVersion } from "./lib/wix-version.mjs";

const USAGE =
  "usage: node scripts/release/wix-version.mjs <tag> [--config <path/to/tauri.windows.conf.json>]";

let parsed;
try {
  parsed = parseArgs({
    options: { config: { type: "string" } },
    allowPositionals: true,
  });
} catch (error) {
  console.error(`::error::${String(error.message ?? error)}`);
  console.error(USAGE);
  process.exit(1);
}

const tag = parsed.positionals[0];
if (!tag) {
  console.error(USAGE);
  process.exit(1);
}

const result = wixVersion(tag);
if (!result.ok) {
  console.error(`::error::${result.error}`);
  process.exit(1);
}

// Tauri merges `tauri.windows.conf.json` into tauri.conf.json on Windows only,
// so the derived version never leaks into the other installers.
if (parsed.values.config) {
  mkdirSync(dirname(parsed.values.config), { recursive: true });
  writeFileSync(parsed.values.config, `${JSON.stringify(wixConfig(result.version), null, 2)}\n`);
}

console.log(result.version);
