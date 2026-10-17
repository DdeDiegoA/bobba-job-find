import type { AuditEntry } from "@/lib/ipc/bindings";

/**
 * A one-line, human summary of an entry's `detail_json` (S9.9): the rule and
 * reason behind an Auto decision, the from → to of a Stage change, or the
 * Source and warning version of an opt-in. The raw JSON stays available beside
 * it; this only decides what the row reads like at a glance.
 */
export function entrySummary(entry: AuditEntry): string {
  const detail = parseDetail(entry.detail_json);
  switch (entry.kind) {
    case "decision": {
      const rule = text(detail.rule) || text(detail.rule_id);
      const note = text(detail.note);
      return [rule, note].filter(Boolean).join(" · ");
    }
    case "stage_change": {
      const from = detail.from === null || detail.from === undefined ? "—" : text(detail.from);
      const to = text(detail.to);
      const reason = text(detail.reason);
      return [`${from} → ${to}`, reason].filter(Boolean).join(" · ");
    }
    case "optin": {
      const name = text(detail.source_name) || text(detail.source_id);
      const version = text(detail.warning_version);
      return [name, version].filter(Boolean).join(" · ");
    }
    default:
      return text(detail.reason) || text(detail.note);
  }
}

function parseDetail(json: string): Record<string, unknown> {
  try {
    const parsed: unknown = JSON.parse(json);
    return typeof parsed === "object" && parsed !== null ? (parsed as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}

function text(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number") return String(value);
  return "";
}
