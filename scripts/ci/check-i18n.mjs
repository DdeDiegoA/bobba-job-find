#!/usr/bin/env node
// CI gate for AD-21: locales/{en,es}/<namespace>.json must expose the same
// namespaces and the same keys in both languages. Missing keys fail the build.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const ROOT = "locales";
const namespaces = (lang) =>
  new Set(readdirSync(join(ROOT, lang)).filter((f) => f.endsWith(".json")));

// Flatten nested messages to dotted key paths. Arrays are leaves (ICU values).
const keys = (obj, prefix = "") =>
  Object.entries(obj).flatMap(([k, v]) =>
    v && typeof v === "object" && !Array.isArray(v)
      ? keys(v, `${prefix}${k}.`)
      : [`${prefix}${k}`],
  );

const read = (lang, ns) => JSON.parse(readFileSync(join(ROOT, lang, ns), "utf8"));
const errs = [];
const en = namespaces("en");
const es = namespaces("es");

for (const f of en) if (!es.has(f)) errs.push(`locales/es/${f}: namespace missing (exists in en)`);
for (const f of es) if (!en.has(f)) errs.push(`locales/en/${f}: namespace missing (exists in es)`);

for (const f of en) {
  if (!es.has(f)) continue;
  const a = new Set(keys(read("en", f)));
  const b = new Set(keys(read("es", f)));
  for (const k of a) if (!b.has(k)) errs.push(`locales/es/${f}: missing key "${k}"`);
  for (const k of b) if (!a.has(k)) errs.push(`locales/en/${f}: missing key "${k}"`);
}

if (errs.length) {
  for (const e of errs) console.error(`::error::${e}`);
  process.exit(1);
}
console.log(`i18n ok: ${en.size} namespaces, EN/ES keys match`);
