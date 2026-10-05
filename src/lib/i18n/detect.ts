/**
 * Locale resolution (S1.4, AC1/AC2). Pure and side-effect free so it is cheap
 * to unit-test; `index.ts` wires it into i18next.
 */
export const SUPPORTED_LOCALES = ["en", "es"] as const;

export type Locale = (typeof SUPPORTED_LOCALES)[number];

/** map an arbitrary BCP-47 tag to a shipped locale: `es*` → `es`, else `en`. */
export function resolveLocale(tag: string | null | undefined): Locale {
  return tag?.toLowerCase().startsWith("es") ? "es" : "en";
}

/**
 * Pick the startup locale: an explicit persisted choice wins, otherwise the OS
 * locale (the Tauri webview reports it as `navigator.language`).
 */
export function pickLocale(
  stored: string | null | undefined,
  osTag: string | null | undefined,
): Locale {
  return stored === "es" || stored === "en" ? stored : resolveLocale(osTag);
}

/** Locale of the running OS/webview. */
export function osLocale(): Locale {
  return resolveLocale(typeof navigator === "undefined" ? undefined : navigator.language);
}
