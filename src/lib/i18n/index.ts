import i18n from "i18next";
import ICU from "i18next-icu";
import { initReactI18next } from "react-i18next";
import { type Locale, osLocale, pickLocale, SUPPORTED_LOCALES } from "./detect";

/**
 * Namespaces are discovered by file name (AD-21): one JSON file per namespace
 * under `locales/{en,es}/`. Adding a namespace = dropping a file in; no
 * registration edit is needed.
 */
const catalogs = import.meta.glob<{ default: Record<string, unknown> }>("/locales/*/*.json", {
  eager: true,
});

/**
 * Core error keys live one file per area under `errors/` (CH7) so two slices never
 * append to the same catalog. They stay in the default namespace with their flat
 * dotted key, so they are merged into `common` and `t("errors.<area>.<x>")` resolves.
 */
const errorFragments = import.meta.glob<{ default: Record<string, unknown> }>(
  "/locales/*/errors/*.json",
  { eager: true },
);

export const defaultNS = "common";

const resources: Record<string, Record<string, Record<string, unknown>>> = {};
for (const [path, mod] of Object.entries(catalogs)) {
  const match = /\/locales\/([^/]+)\/([^/]+)\.json$/.exec(path);
  if (!match) continue;
  const [, lng, ns] = match;
  resources[lng] ??= {};
  resources[lng][ns] = mod.default;
}
for (const [path, mod] of Object.entries(errorFragments)) {
  const match = /\/locales\/([^/]+)\/errors\/[^/]+\.json$/.exec(path);
  if (!match) continue;
  const [, lng] = match;
  resources[lng] ??= {};
  resources[lng][defaultNS] = { ...resources[lng][defaultNS], ...mod.default };
}

export const namespaces = Object.keys(resources.en ?? {});

/** Persisted UI language (AC2). The settings table (S1.7) may own this later. */
const STORAGE_KEY = "boba.locale";

function storedLocale(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

void i18n
  .use(ICU)
  .use(initReactI18next)
  .init({
    lng: pickLocale(storedLocale(), osLocale()),
    fallbackLng: "en",
    supportedLngs: [...SUPPORTED_LOCALES],
    defaultNS,
    ns: namespaces.length > 0 ? namespaces : [defaultNS],
    resources,
    interpolation: { escapeValue: false },
  });

/** The active UI locale. */
export function currentLocale(): Locale {
  return i18n.language === "es" ? "es" : "en";
}

/** Switch the UI language live (no restart) and persist the choice (AC2). */
export function setLocale(lng: Locale): void {
  void i18n.changeLanguage(lng);
  try {
    localStorage.setItem(STORAGE_KEY, lng);
  } catch {
    // Storage disabled (e.g. hardening): the switch still applies for this run.
  }
}

export default i18n;
