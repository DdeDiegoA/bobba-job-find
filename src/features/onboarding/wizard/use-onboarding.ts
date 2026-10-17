import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import { commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";
import { clearOnboarded, isOnboarded, markOnboarded } from "./progress";

/** The core's completion state; one cached query for the whole launch. */
export const onboardingStatusKey = ["onboarding", "status"] as const;

/**
 * Whether setup finished, read from the core (S3.8): `onboarding_status` is a
 * `setting` row, so clearing the webview storage cannot replay onboarding, and
 * Reset everything (which wipes the store) makes it run again.
 *
 * The core's answer refreshes the local mirror on every launch. `null` means
 * the core has not answered yet; a locked store keeps the mirror's answer, so
 * the pre-S3.8 behaviour (onboarding remembered on this machine) survives.
 */
export function useOnboardingGate(): boolean | null {
  const status = useQuery({
    queryKey: onboardingStatusKey,
    queryFn: () => commands.onboardingStatus().then(unwrap),
    // A locked store answers with an error; the mirror is the fallback then.
    retry: false,
  });

  useEffect(() => {
    if (!status.data) return;
    if (status.data.completed) markOnboarded();
    else clearOnboarded();
  }, [status.data]);

  if (status.isPending) return null;
  // A fresh core answer wins. While a refetch is in flight (the launch that
  // just finished setup) or when the core cannot answer (locked store), the
  // mirror decides — it was set when the wizard finished on this machine.
  if (status.data && !status.isFetching) return status.data.completed;
  return isOnboarded();
}
