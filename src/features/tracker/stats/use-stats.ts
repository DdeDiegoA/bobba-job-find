import { useQuery } from "@tanstack/react-query";
import { commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/**
 * The funnel (S9.9, FR-97): computed in the core from the Tracker's own rows.
 * One query, so the cards, the Stage table and the breakdowns never disagree.
 */
export const funnelKey = ["stats-funnel"] as const;

export function useFunnelStats() {
  return useQuery({
    queryKey: funnelKey,
    queryFn: () => commands.statsFunnel().then(unwrap),
  });
}
