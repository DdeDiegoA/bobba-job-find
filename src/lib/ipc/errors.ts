import type { AppError } from "./bindings";

/** Envelope returned by a specta command (`ErrorHandlingMode::Result`). */
export type CommandResult<T> = { status: "ok"; data: T } | { status: "error"; error: AppError };

/** i18n key shown when an error has no resolvable `message_key`. */
export const UNKNOWN_ERROR_KEY = "errors.unknown";

/** Narrow an unknown rejection payload to an [`AppError`]. */
export function isAppError(value: unknown): value is AppError {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return typeof candidate.code === "string" && typeof candidate.message_key === "string";
}

/**
 * Resolve an `AppError` to display text via i18n (AD-3). Only `message_key` is
 * translated; the raw key, `code` and `details` never reach the screen.
 */
export function renderAppError(error: AppError, t: (key: string) => string): string {
  const key = error.message_key || UNKNOWN_ERROR_KEY;
  const text = t(key);
  // i18next returns the key itself when it is missing: fall back, never the raw key.
  return text === key ? t(UNKNOWN_ERROR_KEY) : text;
}

/** Unwrap a specta command result, throwing the typed `AppError` on failure. */
export function unwrap<T>(result: CommandResult<T>): T {
  if (result.status === "ok") return result.data;
  throw result.error;
}
