import { useTranslation } from "react-i18next";
import type { ReasonView } from "@/lib/ipc/bindings";

/** Minutes of the day as `HH:MM`, the way the active hours are shown (FR-83). */
export function minuteClock(minute: number): string {
  const hours = Math.floor(minute / 60)
    .toString()
    .padStart(2, "0");
  const minutes = (minute % 60).toString().padStart(2, "0");
  return `${hours}:${minutes}`;
}

/** A stable key per reason: its code plus the values that make it specific. */
function reasonKey(reason: ReasonView): string {
  const params = Object.entries(reason.params)
    .map(([key, value]) => `${key}=${value}`)
    .join(",");
  return `${reason.code}|${params}`;
}

/** The params as the copy interpolates them: clocks for the hour bounds. */
function displayParams(reason: ReasonView): Record<string, string> {
  const params: Record<string, string> = { ...reason.params };
  for (const key of ["start_minute", "end_minute"] as const) {
    const value = params[key];
    if (value !== undefined) {
      params[key] = minuteClock(Number(value));
    }
  }
  return params;
}

/**
 * The reasons the core gave, word for word (AC2): the code names a leaf under
 * `applying:reason.` and the core's params interpolate into it. This component
 * never adds, reorders or summarises a reason.
 */
export function DecisionReasons({ reasons }: { reasons: ReasonView[] }) {
  const { t } = useTranslation("applying");
  return (
    <ul className="flex list-disc flex-col gap-1 pl-5 text-sm text-fg-muted">
      {reasons.map((reason) => (
        <li key={reasonKey(reason)}>
          {t(`applying:reason.${reason.code}`, displayParams(reason))}
        </li>
      ))}
    </ul>
  );
}
