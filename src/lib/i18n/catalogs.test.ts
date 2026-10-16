import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import i18n, { defaultNS, setLocale } from "./index";

/**
 * CH7: core error keys live in per-area fragments `locales/<lang>/errors/<area>.json`
 * but stay in the default `common` namespace (the key is still the flat dotted
 * `errors.<area>.<x>`), so the loader must merge them and `t(key)` must resolve in
 * both languages. The test reads the fragments from disk: a new fragment is checked
 * without touching this file.
 */
const LANGS = ["en", "es"] as const;

const fragmentDir = (lang: string) => join(process.cwd(), "locales", lang, "errors");

const fragments = (lang: string): Record<string, string>[] => {
  const dir = fragmentDir(lang);
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((f) => f.endsWith(".json"))
    .map((f) => JSON.parse(readFileSync(`${dir}/${f}`, "utf8")) as Record<string, string>);
};

describe("error fragments (CH7)", () => {
  it("ships the same fragments for EN and ES", () => {
    expect(readdirSync(fragmentDir("en")).sort()).toEqual(readdirSync(fragmentDir("es")).sort());
    expect(fragments("en").length).toBeGreaterThan(0);
  });

  it("names each key after its own area", () => {
    for (const lang of LANGS) {
      const dir = fragmentDir(lang);
      for (const file of readdirSync(dir).filter((f) => f.endsWith(".json"))) {
        const area = file.slice(0, -5);
        for (const key of Object.keys(JSON.parse(readFileSync(`${dir}/${file}`, "utf8")))) {
          const own = key === `errors.${area}` || key.startsWith(`errors.${area}.`);
          expect(own, `${lang}/${file}: ${key}`).toBe(true);
        }
      }
    }
  });

  it("resolves every fragment key in its own language", () => {
    const before = i18n.language;
    try {
      for (const lang of LANGS) {
        setLocale(lang);
        for (const fragment of fragments(lang)) {
          for (const [key, value] of Object.entries(fragment)) {
            expect(i18n.t(key), `${lang}: ${key}`).toBe(value);
          }
        }
      }
    } finally {
      setLocale(before === "es" ? "es" : "en");
    }
  });

  it("keeps the default namespace for the fragments", () => {
    expect(defaultNS).toBe("common");
    expect(i18n.t("errors.unknown")).not.toBe("errors.unknown");
  });
});
