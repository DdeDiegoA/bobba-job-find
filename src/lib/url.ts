/**
 * True when `value` is an absolute `https://` URL.
 *
 * A posting's apply URL is untrusted source data (AD-7): the Rust adapters
 * refuse `javascript:`, `data:` and `http:` before a posting is stored (the
 * S4.5 rule), and this is its UI-side twin — the last guard before a string
 * becomes a clickable link in the webview.
 */
export function isHttpsUrl(value: string | null | undefined): value is string {
  if (!value) return false;
  const trimmed = value.trim();
  if (!trimmed.toLowerCase().startsWith("https://")) return false;
  if (/\s/.test(trimmed)) return false;
  try {
    const url = new URL(trimmed);
    return url.protocol === "https:" && url.hostname.length > 0;
  } catch {
    return false;
  }
}
