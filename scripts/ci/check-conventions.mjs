#!/usr/bin/env node
// CH13 — repository conventions gate.
//
// Static checks for the structural rules that the compiler and `cargo test`
// cannot see. They run in CI's `hygiene` job and in `.githooks/pre-commit`.
// Each check is cheap, deterministic and does not build anything.
//
// Rules enforced here (docs/contributing/conventions.md):
//   1. Tauri event names never contain `.` (CH10 regression).
//   2. `errors.*` keys never live in `common.json`; every error area file is
//      listed in `ERROR_FRAGMENTS` and EN/ES carry the same keys (CH7).
//   3. Migration numbers under boba-store/migrations are unique and contiguous.
//   4. Tauri commands are registered only in `ipc/command_list.rs`, and every
//      `#[tauri::command]` function appears there.
//   5. Ports are mounted only in `wiring/port_list.rs` (ADR-012), except a
//      module's `mount()` helper that `port_list.rs` itself calls.
//   6. The adapter dependency rule has a test (`src-tauri/tests/
//      dependency_rules.rs`); this gate only makes sure that test is present.
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = process.cwd();
const errors = [];
const fail = (message) => errors.push(message);

const read = (path) => readFileSync(join(ROOT, path), "utf8");

/** Recursively collect files under `dir` whose name matches `filter`. */
function walk(dir, filter) {
  const out = [];
  const abs = join(ROOT, dir);
  if (!existsSync(abs)) return out;
  for (const entry of readdirSync(abs)) {
    const rel = join(dir, entry);
    const stat = statSync(join(ROOT, rel));
    if (stat.isDirectory()) {
      if (entry === "target" || entry === "node_modules") continue;
      out.push(...walk(rel, filter));
    } else if (filter(entry)) {
      out.push(rel);
    }
  }
  return out;
}

const rustFiles = () => [
  ...walk("src-tauri/src", (f) => f.endsWith(".rs")),
  ...walk("src-tauri/crates", (f) => f.endsWith(".rs")),
];

/// 1. Event names -------------------------------------------------------------

const EVENT_ATTR = /#\[tauri_specta\(\s*event_name\s*=\s*"([^"]+)"\s*\)\]/g;
const EMIT_LITERAL = /\.emit(?:_all)?\s*\(\s*"([^"]+)"/g;
const EMIT_TO_LITERAL = /\.emit_to\s*\(\s*[^,]+,\s*"([^"]+)"/g;

for (const file of rustFiles()) {
  const src = read(file);
  for (const re of [EVENT_ATTR, EMIT_LITERAL, EMIT_TO_LITERAL]) {
    re.lastIndex = 0;
    for (const match of src.matchAll(re)) {
      const name = match[1];
      if (name.includes(".")) {
        const line = src.slice(0, match.index).split("\n").length;
        fail(`${file}:${line}: event name "${name}" contains '.' (use '<area>:<verb-ed>')`);
      }
    }
  }
}

/// 2. Error catalogs ----------------------------------------------------------

const I18N_MOD = "src-tauri/crates/boba-app/src/i18n/mod.rs";
const LOCALES = ["en", "es"];

/** Flatten a JSON object into dotted keys. */
function flatKeys(value, prefix = "") {
  const keys = [];
  for (const [key, child] of Object.entries(value)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (child && typeof child === "object" && !Array.isArray(child)) {
      keys.push(...flatKeys(child, path));
    } else {
      keys.push(path);
    }
  }
  return keys;
}

for (const lang of LOCALES) {
  const commonPath = `locales/${lang}/common.json`;
  if (!existsSync(join(ROOT, commonPath))) {
    fail(`${commonPath}: missing`);
    continue;
  }
  for (const key of flatKeys(JSON.parse(read(commonPath)))) {
    if (key === "errors" || key.startsWith("errors.")) {
      fail(`${commonPath}: core error key "${key}" belongs in locales/${lang}/errors/<area>.json`);
    }
  }
}

const i18n = read(I18N_MOD);
const fragmentMatch = i18n.match(/error_fragments!\s*\[([\s\S]*?)\]/);
const listedAreas = fragmentMatch
  ? [...fragmentMatch[1].matchAll(/"([a-z0-9_]+)"/g)].map((m) => m[1])
  : [];
if (!fragmentMatch) fail(`${I18N_MOD}: no error_fragments![...] list found`);
if (new Set(listedAreas).size !== listedAreas.length) {
  fail(`${I18N_MOD}: duplicate area in ERROR_FRAGMENTS`);
}

const areasByLang = {};
for (const lang of LOCALES) {
  const dir = `locales/${lang}/errors`;
  const files = existsSync(join(ROOT, dir))
    ? readdirSync(join(ROOT, dir))
        .filter((f) => f.endsWith(".json"))
        .map((f) => f.slice(0, -".json".length))
        .sort()
    : [];
  areasByLang[lang] = files;
  for (const area of files) {
    if (!listedAreas.includes(area)) {
      fail(`locales/${lang}/errors/${area}.json: area "${area}" is not in ERROR_FRAGMENTS (${I18N_MOD})`);
    }
    const keys = flatKeys(JSON.parse(read(`${dir}/${area}.json`)));
    for (const key of keys) {
      if (!key.startsWith(`errors.${area}.`) && key !== `errors.${area}`) {
        fail(`locales/${lang}/errors/${area}.json: key "${key}" must start with "errors.${area}."`);
      }
    }
  }
}
for (const area of listedAreas) {
  for (const lang of LOCALES) {
    if (!areasByLang[lang].includes(area)) {
      fail(`ERROR_FRAGMENTS lists "${area}" but locales/${lang}/errors/${area}.json is missing`);
    }
  }
}
const enAreas = areasByLang.en ?? [];
const esAreas = areasByLang.es ?? [];
for (const area of enAreas.filter((a) => !esAreas.includes(a))) {
  fail(`locales/es/errors/${area}.json: missing (EN has it, ES does not)`);
}
for (const area of esAreas.filter((a) => !enAreas.includes(a))) {
  fail(`locales/en/errors/${area}.json: missing (ES has it, EN does not)`);
}
// Same keys in both languages, so a lookup never falls through.
for (const area of enAreas.filter((a) => esAreas.includes(a))) {
  const enKeys = flatKeys(JSON.parse(read(`locales/en/errors/${area}.json`))).sort();
  const esKeys = flatKeys(JSON.parse(read(`locales/es/errors/${area}.json`))).sort();
  const missingEs = enKeys.filter((k) => !esKeys.includes(k));
  const missingEn = esKeys.filter((k) => !enKeys.includes(k));
  for (const key of missingEs) fail(`locales/es/errors/${area}.json: missing key "${key}"`);
  for (const key of missingEn) fail(`locales/en/errors/${area}.json: missing key "${key}"`);
}

/// 3. Migrations --------------------------------------------------------------

const MIGRATIONS = "src-tauri/crates/boba-store/migrations";
if (!existsSync(join(ROOT, MIGRATIONS))) {
  fail(`${MIGRATIONS}: missing`);
} else {
  const numbers = [];
  for (const file of readdirSync(join(ROOT, MIGRATIONS))) {
    const match = file.match(/^(\d{4})_(.+)\.sql$/);
    if (!match) {
      fail(`${MIGRATIONS}/${file}: expected NNNN_name.sql`);
      continue;
    }
    numbers.push(Number(match[1]));
  }
  numbers.sort((a, b) => a - b);
  const seen = new Set();
  for (const n of numbers) {
    if (seen.has(n)) fail(`${MIGRATIONS}: duplicate migration number ${String(n).padStart(4, "0")}`);
    seen.add(n);
  }
  numbers.forEach((n, index) => {
    if (n !== index + 1) {
      fail(
        `${MIGRATIONS}: migration numbers must run 0001..${String(numbers.length).padStart(4, "0")} ` +
          `without gaps; found ${String(n).padStart(4, "0")} at position ${index + 1}`,
      );
    }
  });
}

/// 4. IPC registration --------------------------------------------------------

const COMMAND_LIST = "src-tauri/src/ipc/command_list.rs";
const commandList = read(COMMAND_LIST);

for (const file of [...walk("src-tauri/src", (f) => f.endsWith(".rs"))]) {
  if (file === COMMAND_LIST) continue;
  const src = read(file);
  if (/collect_commands!\s*\[/.test(src) || /collect_events!\s*\[/.test(src)) {
    fail(`${file}: collect_commands!/collect_events! may only appear in ${COMMAND_LIST}`);
  }
}

const tested = (src) => {
  // Line ranges covered by a `#[cfg(test)]` module, so a test-only command
  // helper is not reported as unregistered.
  const lines = src.split("\n");
  const ranges = [];
  for (let i = 0; i < lines.length; i++) {
    if (!/^\s*#\[cfg\(test\)\]/.test(lines[i])) continue;
    let depth = 0;
    let started = false;
    for (let j = i; j < lines.length; j++) {
      for (const ch of lines[j]) {
        if (ch === "{") {
          depth++;
          started = true;
        } else if (ch === "}") {
          depth--;
        }
      }
      if (started && depth === 0) {
        ranges.push([i, j]);
        break;
      }
    }
  }
  return (line) => ranges.some(([start, end]) => line >= start && line <= end);
};

for (const file of walk("src-tauri/src", (f) => f.endsWith(".rs"))) {
  if (file === COMMAND_LIST) continue;
  const src = read(file);
  const lines = src.split("\n");
  const isTest = tested(src);
  lines.forEach((line, index) => {
    if (!/^\s*#\[tauri::command\]/.test(line) || isTest(index)) return;
    for (let j = index + 1; j < Math.min(index + 10, lines.length); j++) {
      const fn = lines[j].match(/\bfn\s+([a-z0-9_]+)\s*[<(]/);
      if (!fn) continue;
      if (!new RegExp(`::${fn[1]}\\s*,`).test(commandList)) {
        fail(`${file}:${j + 1}: command "${fn[1]}" is not registered in ${COMMAND_LIST}`);
      }
      break;
    }
  });
}

/// 5. Port mounting -----------------------------------------------------------

const PORT_LIST = "src-tauri/src/wiring/port_list.rs";
const portList = read(PORT_LIST);
for (const file of walk("src-tauri/src", (f) => f.endsWith(".rs"))) {
  if (file === PORT_LIST) continue;
  const src = read(file);
  if (!/mount_port::</.test(src)) continue;
  // ADR-012: a slice may ship a `mount()` helper that mounts its own ports,
  // but only when `port_list.rs` calls that helper as one line.
  const module = file
    .replace(/^src-tauri\/src\//, "")
    .replace(/\.rs$/, "")
    .replace(/\/mod$/, "")
    .split("/")
    .join("::");
  const wired = new RegExp(`crate::${module}::mount\\s*\\(`).test(portList);
  if (!wired) {
    fail(
      `${file}: calls mount_port::<...> but ${PORT_LIST} does not call crate::${module}::mount( ` +
        `(mount ports in the composition root, or expose a mount() helper it calls)`,
    );
  }
}

/// 6. Adapter dependency rule -------------------------------------------------

const DEPENDENCY_RULES = "src-tauri/tests/dependency_rules.rs";
if (!existsSync(join(ROOT, DEPENDENCY_RULES))) {
  fail(
    `${DEPENDENCY_RULES}: missing — adapters-must-not-depend-on-adapters is enforced by that cargo test`,
  );
}

/// Report ---------------------------------------------------------------------

if (errors.length) {
  for (const message of errors) console.error(`::error::${message}`);
  console.error(`conventions: ${errors.length} problem(s)`);
  process.exit(1);
}
console.log(
  "conventions ok: event names, error catalogs, migrations, IPC list, port list, dependency rules",
);
