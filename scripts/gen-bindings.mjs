#!/usr/bin/env node
// Regenerate or check src/lib/ipc/bindings.ts (AD-3, build convention 4).
//
//   node scripts/gen-bindings.mjs          # regenerate + format
//   node scripts/gen-bindings.mjs --check  # exit 1 when stale (CI)
//
// The Rust side (`ipc::tests::export_bindings`) exports the raw tauri-specta
// output; this script formats it with Biome so the committed file is lint-clean
// and deterministic.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repo = join(dirname(fileURLToPath(import.meta.url)), "..");
const check = process.argv.includes("--check");
const generated = join(tmpdir(), "boba-bindings.generated.ts");
const target = "src/lib/ipc/bindings.ts";
const out = join(repo, target);

execFileSync(
  "cargo",
  [
    "test",
    "--manifest-path",
    join(repo, "src-tauri/Cargo.toml"),
    "-p",
    "boba-job-find",
    "--lib",
    "ipc::tests::export_bindings",
    "--",
    "--exact",
    "--nocapture",
  ],
  { cwd: repo, stdio: "inherit" },
);

if (!existsSync(generated)) {
  console.error(`gen-bindings: ${generated} was not written by the export test`);
  process.exit(1);
}

const raw = readFileSync(generated, "utf8");
const formatted = execFileSync("npx", ["biome", "check", "--write", `--stdin-file-path=${target}`], {
  cwd: repo,
  input: raw,
  encoding: "utf8",
});
const committed = existsSync(out) ? readFileSync(out, "utf8") : "";

if (formatted === committed) {
  console.log(`gen-bindings: ${target} is up to date`);
} else if (check) {
  console.error(`gen-bindings: ${target} is stale — run \`node scripts/gen-bindings.mjs\``);
  process.exit(1);
} else {
  writeFileSync(out, formatted);
  console.log(`gen-bindings: wrote ${target}`);
}
