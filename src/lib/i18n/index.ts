import i18n from "i18next";
import ICU from "i18next-icu";
import { initReactI18next } from "react-i18next";

/**
 * Namespaces are discovered by file name (AD-21): one JSON file per namespace
 * under `locales/{en,es}/`. Adding a namespace = dropping a file in; no
 * registration edit is needed. S1.4 layers ICU extraction and the core-side
 * embed on top of this.
 */
const catalogs = import.meta.glob<{ default: Record<string, unknown> }>(
  "/locales/*/*.json",
  { eager: true },
);

const resources: Record<string, Record<string, Record<string, unknown>>> = {};
for (const [path, mod] of Object.entries(catalogs)) {
  const match = /\/locales\/([^/]+)\/([^/]+)\.json$/.exec(path);
  if (!match) continue;
  const [, lng, ns] = match;
  resources[lng] ??= {};
  resources[lng][ns] = mod.default;
}

export const defaultNS = "common";
export const namespaces = Object.keys(resources.en ?? {});

void i18n
  .use(ICU)
  .use(initReactI18next)
  .init({
    lng: "en",
    fallbackLng: "en",
    defaultNS,
    ns: namespaces.length > 0 ? namespaces : [defaultNS],
    resources,
    interpolation: { escapeValue: false },
  });

export default i18n;
