// FX18 — negative tests for the "built but never mounted" guard.
//
// The guard is a static rule inside `check-conventions.mjs`: a feature leaf
// (`src/features/**/index.tsx` with a default export) that no non-test file
// imports is a screen nobody can reach (the beta.2 stub sweep found six of
// them). These tests build tiny fixture trees in a temp dir and call the rule
// directly, so a regression fails here before it reaches the repo.
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { checkLeafGuard } from "./leaf-guard.mjs";

const ALLOWLIST = "scripts/ci/leaf-guard.allowlist.json";

const roots = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

/** Write `files` (path -> text) under a fresh temp root; always an allowlist. */
function fixture(files = {}) {
  const root = mkdtempSync(join(tmpdir(), "leaf-guard-"));
  roots.push(root);
  const all = { [ALLOWLIST]: JSON.stringify({ allow: [] }), ...files };
  for (const [path, text] of Object.entries(all)) {
    const abs = join(root, path);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, text);
  }
  return root;
}

const LEAF = `export default function Alpha() {
  return <h1>Alpha</h1>;
}
`;

const ROUTES = `import Alpha from "@/features/alpha";

export const routes = [{ path: "/", element: <Alpha /> }];
`;

const problemsFor = (root, path) => checkLeafGuard(root).filter((problem) => problem.includes(path));

describe("leaf guard: unmounted feature leaves", () => {
  it("flags a leaf that no non-test file imports, naming the file", () => {
    const root = fixture({ "src/features/alpha/index.tsx": LEAF });

    expect(problemsFor(root, "src/features/alpha/index.tsx")).toHaveLength(1);
  });

  it("accepts a leaf mounted by a static import", () => {
    const root = fixture({
      "src/features/alpha/index.tsx": LEAF,
      "src/app/routes.tsx": ROUTES,
    });

    expect(checkLeafGuard(root)).toEqual([]);
  });

  it("accepts a leaf mounted by a lazy route import", () => {
    const root = fixture({
      "src/features/alpha/index.tsx": LEAF,
      "src/app/routes.tsx": `import { lazy } from "react";
const Alpha = lazy(() => import("@/features/alpha"));
export const routes = [{ path: "/", element: <Alpha /> }];
`,
    });

    expect(checkLeafGuard(root)).toEqual([]);
  });

  it("does not count a test-only import as mounting the leaf", () => {
    const root = fixture({
      "src/features/alpha/index.tsx": LEAF,
      "src/features/alpha/index.test.tsx": `import Alpha from "./index";
export const x = Alpha;
`,
    });

    expect(problemsFor(root, "src/features/alpha/index.tsx")).toHaveLength(1);
  });

  it("skips files without a default export (shared modules, not leaves)", () => {
    const root = fixture({
      "src/features/alpha/index.tsx": "export function helper() {}\n",
    });

    expect(checkLeafGuard(root)).toEqual([]);
  });
});

describe("leaf guard: allowlist", () => {
  it("exempts an entry that carries a reason", () => {
    const root = fixture({
      "src/features/alpha/index.tsx": LEAF,
      [ALLOWLIST]: JSON.stringify({
        allow: [{ path: "src/features/alpha/index.tsx", reason: "Mounted by the native menu" }],
      }),
    });

    expect(checkLeafGuard(root)).toEqual([]);
  });

  it("rejects an entry with no reason", () => {
    const root = fixture({
      "src/features/alpha/index.tsx": LEAF,
      [ALLOWLIST]: JSON.stringify({ allow: [{ path: "src/features/alpha/index.tsx", reason: "" }] }),
    });

    const problems = checkLeafGuard(root);
    expect(problems.some((problem) => problem.includes("reason"))).toBe(true);
  });

  it("rejects an entry that is no longer needed (stale allowlist)", () => {
    const root = fixture({
      "src/features/alpha/index.tsx": LEAF,
      "src/app/routes.tsx": ROUTES,
      [ALLOWLIST]: JSON.stringify({
        allow: [{ path: "src/features/alpha/index.tsx", reason: "Was orphaned in beta.2" }],
      }),
    });

    expect(checkLeafGuard(root).some((problem) => problem.includes("stale"))).toBe(true);
  });
});

describe("leaf guard: placeholder pages and dead-interaction markers", () => {
  it("flags a placeholder page whose body is a lone h1", () => {
    const root = fixture({
      "src/features/alpha/index.tsx": `export default function Alpha() {
  return <h1>Index</h1>;
}
`,
      "src/app/routes.tsx": ROUTES,
    });

    expect(problemsFor(root, "src/features/alpha/index.tsx")).toHaveLength(1);
  });

  it("flags a 'no handler yet' marker on an interactive element", () => {
    const root = fixture({
      "src/features/alpha/index.tsx": `import { Button } from "@/components/ui/button";

export default function Alpha() {
  // ponytail: no handler yet
  return <Button>Run now</Button>;
}
`,
      "src/app/routes.tsx": ROUTES,
    });

    expect(problemsFor(root, "src/features/alpha/index.tsx")).toHaveLength(1);
  });
});

describe("leaf guard: the real repository", () => {
  it("is green on the current tree", () => {
    expect(checkLeafGuard(process.cwd())).toEqual([]);
  });

  it("runs inside the conventions gate", () => {
    const run = spawnSync(process.execPath, ["scripts/ci/check-conventions.mjs"], {
      cwd: process.cwd(),
      encoding: "utf8",
    });

    expect(run.status, `${run.stdout}\n${run.stderr}`).toBe(0);
  });
});
