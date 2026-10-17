import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/**
 * The network-activity surface (S10.4, NFR-1, FR-115): one query for the
 * grouped destinations, one for the FX setting, and one mutation that turns
 * conversion on (which fetches a rate in the core) or off. Every decision lives
 * in the core; this file carries no rules.
 */
export const networkActivityKey = ["network", "activity"] as const;
export const fxSettingsKey = ["network", "fx"] as const;
export const updateSettingsKey = ["network", "update"] as const;

/** Every destination contacted or configured, grouped by purpose (AC1). */
export function useNetworkActivity() {
  return useQuery({
    queryKey: networkActivityKey,
    queryFn: () => commands.networkActivity().then(unwrap),
  });
}

/** Whether conversions are on, and the rates behind them (AC2). */
export function useFxSettings() {
  return useQuery({
    queryKey: fxSettingsKey,
    queryFn: () => commands.fxSettings().then(unwrap),
  });
}

/** Turn salary conversion on (fetching today's rates once) or off. */
export function useSetFx() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (enabled: boolean) => commands.fxSetEnabled(enabled).then(unwrap),
    onSuccess: (settings) => {
      queryClient.setQueryData(fxSettingsKey, settings);
      // Turning FX on adds (or removes) its destination in the view.
      void queryClient.invalidateQueries({ queryKey: networkActivityKey });
    },
  });
}

/** Whether update checks are on (S10.5, NFR-1). Off on a fresh install. */
export function useUpdateSettings() {
  return useQuery({
    queryKey: updateSettingsKey,
    // Reading the row cannot fail (a locked store reads as "off"), so the
    // command returns the value directly instead of a typed error.
    queryFn: () => commands.updateSettings(),
  });
}

/** Opt in to (or out of) update checks. Turning them on checks immediately. */
export function useSetUpdateChecks() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (enabled: boolean) => commands.updateSetEnabled(enabled).then(unwrap),
    onSuccess: (settings) => {
      queryClient.setQueryData(updateSettingsKey, settings);
      // Turning checks on adds (or removes) the update destination.
      void queryClient.invalidateQueries({ queryKey: networkActivityKey });
    },
  });
}
