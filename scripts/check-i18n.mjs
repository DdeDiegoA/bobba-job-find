#!/usr/bin/env node
// CI gate for AD-21 (S1.4). The shared catalogs under `locales/{en,es}/` are the
// single source of user-facing strings, for both the UI (i18next) and the core
// (embedded at compile time). This check fails when:
//
//   1. a namespace file exists in one language but not the other,
//   2. a key exists in one language but not the other (`missing key`),
//   3. a value is not valid ICU message syntax (`ICU syntax error`),
//   4. a `message_key` the core emits has no catalog entry (AC4), or
//   5. a catalog key is referenced by no source file (`unused key`).
//
// Run from the repo root: `node scripts/check-i18n.mjs` (CI runs the same).

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import i18next from "i18next";
import ICU from "i18next-icu";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const LOCALES = join(ROOT, "locales");
const DEFAULT_NS = "common";
const LANGS = ["en", "es"];
const SKIP_DIRS = new Set(["target", "node_modules", ".git", "dist", ".hermad"]);

const errors = [];
const fail = (message) => errors.push(message);

/** Recursively collect files under `dir` ending in one of `exts`. */
const walk = (dir, exts, out = []) => {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (!SKIP_DIRS.has(entry.name)) walk(join(dir, entry.name), exts, out);
    } else if (exts.some((ext) => entry.name.endsWith(ext))) {
      out.push(join(dir, entry.name));
    }
  }
  return out;
};

const readJson = (path) => {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (e) {
    fail(`locales/${path.slice(LOCALES.length + 1)}: ${e.message}`);
    return null;
  }
};

/** Flatten a catalog node to dotted leaf paths; arrays are leaves (ICU values). */
const leafEntries = (value, prefix = "") =>
  Object.entries(value).flatMap(([k, v]) =>
    v && typeof v === "object" && !Array.isArray(v)
      ? leafEntries(v, `${prefix}${k}.`)
      : [[`${prefix}${k}`, v]],
  );

// --- 1/2. Namespace and key parity -----------------------------------------
const namespacesOf = (lang) =>
  new Set(
    readdirSync(join(LOCALES, lang))
      .filter((f) => f.endsWith(".json"))
      .map((f) => f.slice(0, -5)),
  );

const enNs = namespacesOf("en");
const esNs = namespacesOf("es");
for (const ns of enNs) if (!esNs.has(ns)) fail(`locales/es/${ns}.json: namespace missing (in en)`);
for (const ns of esNs) if (!enNs.has(ns)) fail(`locales/en/${ns}.json: namespace missing (in es)`);

// catalog[lang][ns] = Map(dotted key -> raw value)
const catalog = {};
for (const lang of LANGS) {
  catalog[lang] = {};
  for (const ns of namespacesOf(lang)) {
    const json = readJson(join(LOCALES, lang, `${ns}.json`));
    if (json) catalog[lang][ns] = new Map(leafEntries(json));
  }
}

for (const ns of enNs) {
  if (!esNs.has(ns)) continue;
  const en = catalog.en[ns] ?? new Map();
  const es = catalog.es[ns] ?? new Map();
  for (const key of en.keys()) if (!es.has(key)) fail(`locales/es/${ns}.json: missing key "${key}"`);
  for (const key of es.keys()) if (!en.has(key)) fail(`locales/en/${ns}.json: missing key "${key}"`);
}

// --- 3. ICU syntax ----------------------------------------------------------
// i18next-icu bundles IntlMessageFormat and exposes it on the i18next instance;
// its constructor throws on a parse error (the runtime formatter silently falls
// back to raw text, so t() alone would not catch it).
await i18next.use(ICU).init({ lng: "en", resources: {} });
const IntlMessageFormat = i18next.IntlMessageFormat;
if (typeof IntlMessageFormat !== "function") {
  fail("i18next-icu did not expose IntlMessageFormat; cannot validate ICU syntax");
} else {
  for (const lang of LANGS) {
    for (const [ns, keys] of Object.entries(catalog[lang] ?? {})) {
      for (const [key, value] of keys) {
        if (typeof value !== "string") continue;
        try {
          new IntlMessageFormat(value, lang);
        } catch (e) {
          fail(`locales/${lang}/${ns}.json: ICU syntax error in "${key}": ${e.message}`);
        }
      }
    }
  }
}

// --- 4/5. Core-emitted keys and unused keys --------------------------------
// Keys the core emits as `message_key` (Rust), skipping test modules (throwaway
// keys); test modules conventionally sit at the end of a file.
const CORE_KEY = /AppError::[a-z_]+\(\s*"([^"]+)"|message_key:\s*(?:Some\()?\s*"([^"]+)"/g;
// Keys returned by a const `message_key()`-style fn: `=> "errors.…"` arms.
// S2.2 added `FailureReason::message_key()`; the pattern catches any fn whose
// body maps variants to i18n keys.
const FN_KEY = /=>\s*"((?:errors|common|ui|agent)\.[^"]+)"/g;
const coreKeys = new Set();
for (const file of walk(join(ROOT, "src-tauri"), [".rs"])) {
  const code = readFileSync(file, "utf8");
  const testAt = code.search(/^\s*#\[cfg\(test\)\]/m);
  const production = testAt === -1 ? code : code.slice(0, testAt);
  for (const m of production.matchAll(CORE_KEY)) coreKeys.add(m[1] ?? m[2]);
  // Only inside `fn …message_key(`, so unrelated `=> "…"` matches never count.
  const fnBody = production.match(/fn\s+message_key[\s\S]*?\n    \}/);
  if (fnBody) {
    for (const m of fnBody[0].matchAll(FN_KEY)) coreKeys.add(m[1]);
  }
}

const splitKey = (raw) => (raw.includes(":") ? raw.split(":", 2) : [DEFAULT_NS, raw]);

// Keys referenced in the UI. Test files are skipped so a key used only in tests
// counts as unused.
const UI_KEY =
  /\bt\(\s*["']([^"']+)["']|i18nKey:\s*["']([^"']+)["']|message_key:\s*["']([^"']+)["']|UNKNOWN_ERROR_KEY\s*=\s*["']([^"']+)["']/g;
// A namespaced key written as a plain string literal, wherever it appears in
// production `src/**` (labels live in data: `labelKey: "shell:nav.today"`,
// consumed later as `t(item.labelKey)`). Mirrors the Rust literal scan.
const LITERAL_KEY = new RegExp(
  `["'\`]((?:${[...enNs].join("|")}):[A-Za-z0-9_.-]+)["'\`]`,
  "g",
);
// A template literal's static prefix uses its whole subtree:
// t(`shell:sidebar.theme.${theme}`) → every `sidebar.theme.*` leaf.
const TEMPLATE_KEY = /\bt\(\s*`([^`$]*?)\$\{/g;
const usedKeys = new Set(coreKeys);
for (const file of walk(join(ROOT, "src"), [".ts", ".tsx"])) {
  if (/\.(test|spec)\.|\.d\.ts$|bindings\.ts$/.test(file)) continue;
  const code = readFileSync(file, "utf8");
  for (const m of code.matchAll(UI_KEY)) usedKeys.add(m[1] ?? m[2] ?? m[3] ?? m[4]);
  for (const m of code.matchAll(LITERAL_KEY)) usedKeys.add(m[1]);
  for (const m of code.matchAll(TEMPLATE_KEY)) {
    const [ns, prefix] = splitKey(m[1]);
    for (const lang of LANGS)
      for (const key of catalog[lang]?.[ns]?.keys() ?? [])
        if (key.startsWith(prefix)) usedKeys.add(`${ns}:${key}`);
  }
}

// AC4: every emitted `message_key` resolves in every language.
for (const raw of coreKeys) {
  const [ns, key] = splitKey(raw);
  for (const lang of LANGS) {
    if (!(catalog[lang]?.[ns]?.has(key) ?? false)) {
      fail(`locales/${lang}/${ns}.json: emitted message_key "${raw}" has no entry`);
    }
  }
}

// Unused: a catalog key must be referenced somewhere. A bare `t("key")` (no
// namespace) counts in every namespace, so a wrong-namespace match never fails
// the build — only genuinely dead keys do.
const qualified = new Set([...usedKeys].filter((k) => k.includes(":")));
const bare = new Set([...usedKeys].filter((k) => !k.includes(":")));
for (const lang of LANGS) {
  for (const [ns, keys] of Object.entries(catalog[lang] ?? {})) {
    for (const key of keys.keys()) {
      if (!bare.has(key) && !qualified.has(`${ns}:${key}`)) {
        fail(`locales/${lang}/${ns}.json: unused key "${key}"`);
      }
    }
  }
}

// --- Report -----------------------------------------------------------------
if (errors.length > 0) {
  for (const message of new Set(errors)) console.error(`::error::${message}`);
  process.exit(1);
}
console.log(`i18n ok: ${enNs.size} namespaces, EN/ES aligned, ICU valid, no unused keys`);
