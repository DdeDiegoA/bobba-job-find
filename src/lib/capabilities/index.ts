import { useQuery } from "@tanstack/react-query";
import { type CapabilitySnapshot, type CapabilityState, commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/**
 * Capability gating for AI-only actions (S3.3, FR-4, AC4).
 *
 * The core decides — `boba_app::capabilities` owns the rules and the probe
 * supplies the lockdown verdict — and the UI only renders the answer. Every
 * feature slice reads a capability through [`useCapability`] and wraps the
 * action in `Gated` instead of re-implementing a disabled state.
 *
 * Ids mirror `Capability::ALL` in `boba-app/src/capabilities`; a capability the
 * core does not report reads as unavailable, so a typo or an older core never
 * enables an action that could not run.
 */
export const CAPABILITY_IDS = [
  "sources",
  "dedup",
  "scoring",
  "tracker",
  "applications_manual",
  "profile_edit",
  "profile_extraction",
  "profile_gaps",
  "profile_translation",
  "posting_extraction",
  "tailoring",
  "application_review",
  "score_narration",
  "answer_questions",
  "email_classification",
] as const;

export type CapabilityId = (typeof CAPABILITY_IDS)[number];

/** One capability as a caller reads it. */
export interface CapabilityAccess {
  available: boolean;
  /** Why it is off (AC2, AC3); `null` while load state or id is unknown. */
  reason: CapabilityState["reason"];
  /** No working Agent CLI: Limited mode (FR-4). */
  limited: boolean;
  /** The snapshot is still loading: controls stay disabled, never clickable. */
  isLoading: boolean;
}

/** The snapshot query. One per session: the probe runs bounded subprocesses. */
function useCapabilitiesQuery() {
  return useQuery({
    queryKey: ["capabilities", "snapshot"],
    queryFn: () => commands.capabilitiesSnapshot().then(unwrap),
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  });
}

/** The whole snapshot, for surfaces that render a Limited-mode banner. */
export function useCapabilities() {
  return useCapabilitiesQuery();
}

/** One capability's state in a loaded snapshot; unknown ids fail closed. */
export function capabilityOf(snapshot: CapabilitySnapshot, id: string): CapabilityState {
  return (
    snapshot.capabilities.find((capability) => capability.id === id) ?? {
      id,
      available: false,
      reason: null,
    }
  );
}

/**
 * Whether `id` may run now, and if not, why (AC2–AC4).
 *
 * Unloaded, failed or unknown all read as unavailable without a reason: the
 * action is disabled but nothing claims to know why, so no misleading CTA ever
 * appears (EXPERIENCE §9.15: never a clickable control that fails later).
 */
export function useCapability(id: CapabilityId): CapabilityAccess {
  const query = useCapabilitiesQuery();
  if (query.data === undefined) {
    return { available: false, reason: null, limited: false, isLoading: query.isPending };
  }
  const found = capabilityOf(query.data, id);
  return {
    available: found.available,
    reason: found.reason,
    limited: query.data.limited,
    isLoading: false,
  };
}
