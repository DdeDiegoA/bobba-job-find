// FX18 — the "built but never mounted" guard.
//
// The beta.2 stub sweep found six screens that shipped `index.tsx` + a test but
// were never imported by a route or a container (the beta.2 Connections stub,
// the Today Needs-you panel, the Disclosures page, the Kill Switch and the
// session opt-in). Each leaf's own test passed because it rendered the leaf
// alone. This module is the static half of the guard (the other half is
// `src/app/containers.test.tsx`, which renders every container for real):
//
//   1. A feature leaf (`src/features/**/index.tsx` with a default export) that
//      no non-test file imports is a screen nobody can reach.
//   2. A page whose body is a lone `<h1>Index</h1>` (or "Placeholder") is a stub.
//   3. A "ponytail" / "no handler yet" marker on an interactive element marks a
//      control that ships and does nothing (the old Run now button).
//
// Intentional exceptions live in `scripts/ci/leaf-guard.allowlist.json`, one
// entry per leaf, each with a written reason. Stale entries fail the gate: an
// allowlist that outlives its reason is how a blanket ignore starts.
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, extname, join, relative, resolve, sep } from "node:path";

export const ALLOWLIST_PATH = "scripts/ci/leaf-guard.allowlist.json";

const SOURCE_EXTENSIONS = [".ts", ".tsx", ".js", ".jsx"];
const JSX_EXTENSIONS = [".tsx", ".jsx"];
const LEAF_PATH = /^src\/features\/(?:.+\/)?index\.tsx$/;
const TEST_FILE = /\.(?:test|spec)\.[cm]?[jt]sx?$/;
const DEFAULT_EXPORT = /export\s+default\b/;
const PLACEHOLDER = /<h1\b[^>]*>\s*(?:Index|Placeholder|TODO)\s*<\/h1>/;
const INTERACTIVE = /<(?:Button|button|a|input|select|textarea)\b|role="button"/;
const STATIC_IMPORT = /import\s+(?:[^"'`;]*?\s+from\s+)?["']([^"']+)["']/g;
const DYNAMIC_IMPORT = /\bimport\s*\(\s*["']([^"']+)["']\s*\)/g;

const slash = (path) => path.split(sep).join("/");

/** Repo-relative paths of every file under `dir`. */
function walk(root, dir) {
  const out = [];
  const abs = join(root, dir);
  if (!existsSync(abs)) return out;
  for (const entry of readdirSync(abs)) {
    const rel = `${dir}/${entry}`;
    const stat = statSync(join(root, rel));
    if (stat.isDirectory()) {
      if (entry === "node_modules" || entry === "target") continue;
      out.push(...walk(root, rel));
    } else {
      out.push(rel);
    }
  }
  return out;
}

const isTestFile = (rel) => TEST_FILE.test(rel) || rel.startsWith("src/test/");

/**
 * The repo-relative file a specifier points at, or `null` for a package import.
 * `@/` is the `src` alias (vite.config.ts); relative specifiers resolve from the
 * importing file. Extensionless specifiers try the file and its `index`.
 */
function resolveImport(root, importerRel, specifier) {
  let base;
  if (specifier.startsWith("@/")) {
    base = join(root, "src", specifier.slice(2));
  } else if (specifier.startsWith("./") || specifier.startsWith("../")) {
    base = resolve(dirname(join(root, importerRel)), specifier);
  } else {
    return null;
  }

  const candidates = SOURCE_EXTENSIONS.includes(extname(base))
    ? [base]
    : [
        ...SOURCE_EXTENSIONS.map((extension) => base + extension),
        ...SOURCE_EXTENSIONS.map((extension) => join(base, `index${extension}`)),
      ];
  for (const candidate of candidates) {
    if (existsSync(candidate) && statSync(candidate).isFile()) {
      return slash(relative(root, candidate));
    }
  }
  return null;
}

/** Every file mounted by a non-test importer (static, re-export or lazy). */
function mountedFiles(root, files) {
  const mounted = new Set();
  for (const file of files) {
    if (isTestFile(file)) continue;
    const text = readFileSync(join(root, file), "utf8");
    for (const pattern of [STATIC_IMPORT, DYNAMIC_IMPORT]) {
      pattern.lastIndex = 0;
      for (const match of text.matchAll(pattern)) {
        const target = resolveImport(root, file, match[1]);
        if (target) mounted.add(target);
      }
    }
  }
  return mounted;
}

/**
 * The allowlist as `path -> reason`. A missing or unreadable file is itself a
 * problem: a silently ignored allowlist is a blanket ignore.
 */
function readAllowlist(root, problems) {
  const abs = join(root, ALLOWLIST_PATH);
  const allowed = new Map();
  if (!existsSync(abs)) {
    problems.push(`${ALLOWLIST_PATH}: missing — it is the only place an exception may live`);
    return allowed;
  }
  let parsed;
  try {
    parsed = JSON.parse(readFileSync(abs, "utf8"));
  } catch (error) {
    problems.push(`${ALLOWLIST_PATH}: not valid JSON (${error.message})`);
    return allowed;
  }
  const entries = Array.isArray(parsed?.allow) ? parsed.allow : [];
  if (!Array.isArray(parsed?.allow)) {
    problems.push(`${ALLOWLIST_PATH}: expected { "allow": [...] }`);
  }
  for (const entry of entries) {
    const path = typeof entry?.path === "string" ? slash(entry.path) : "";
    const reason = typeof entry?.reason === "string" ? entry.reason.trim() : "";
    if (!path) {
      problems.push(`${ALLOWLIST_PATH}: an entry has no "path"`);
      continue;
    }
    if (!reason) {
      problems.push(`${ALLOWLIST_PATH}: entry for ${path} has no reason`);
      continue;
    }
    allowed.set(path, reason);
  }
  return allowed;
}

/**
 * Run the guard over `root`. Returns human-readable problems, each starting
 * with the offending path (check-conventions prints them as `::error::` lines).
 */
export function checkLeafGuard(root = process.cwd()) {
  const problems = [];
  const allowed = readAllowlist(root, problems);
  const files = walk(root, "src").filter((file) => SOURCE_EXTENSIONS.includes(extname(file)));
  const mounted = mountedFiles(root, files);

  // 1. Leaves nobody mounts.
  const leaves = files.filter((file) => LEAF_PATH.test(file));
  for (const leaf of leaves) {
    if (!DEFAULT_EXPORT.test(readFileSync(join(root, leaf), "utf8"))) continue;
    if (mounted.has(leaf)) {
      if (allowed.has(leaf)) {
        problems.push(
          `${ALLOWLIST_PATH}: stale entry for ${leaf} — it is mounted now; drop the entry`,
        );
      }
      continue;
    }
    if (allowed.has(leaf)) continue;
    problems.push(
      `${leaf}: feature leaf has a default export but no non-test file imports it ` +
        `— mount it in a route or container, or allow it in ${ALLOWLIST_PATH} with a reason`,
    );
  }

  // 2. Placeholder pages.
  for (const file of files) {
    if (!JSX_EXTENSIONS.includes(extname(file)) || isTestFile(file)) continue;
    const lines = readFileSync(join(root, file), "utf8").split("\n");
    lines.forEach((line, index) => {
      if (!PLACEHOLDER.test(line)) return;
      problems.push(
        `${file}:${index + 1}: placeholder page — a lone <h1>Index</h1> is a stub; ` +
          "build the real screen (and mount its leaves) or delete the file",
      );
    });
  }

  // 3. Dead-interaction markers.
  for (const file of files) {
    if (isTestFile(file)) continue;
    const lines = readFileSync(join(root, file), "utf8").split("\n");
    lines.forEach((line, index) => {
      const marker = /ponytail/i.test(line)
        ? "ponytail"
        : /no handler yet/i.test(line)
          ? "no handler yet"
          : null;
      if (!marker) return;
      // "ponytail" may name the skill in prose; it only fails next to a control.
      const nearby = lines.slice(Math.max(0, index - 2), index + 4).join("\n");
      if (marker === "ponytail" && !INTERACTIVE.test(nearby)) return;
      problems.push(
        `${file}:${index + 1}: "${marker}" marker on an interactive element — ` +
          "a control that ships without a handler must not pass the gate",
      );
    });
  }

  return problems;
}
