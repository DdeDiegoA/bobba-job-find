import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type AutostartStatus, commands } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/**
 * The OS's launch-at-login state (S5.4, FR-44). The core reconciles the stored
 * choice with the OS at every startup; this query reads what the OS says right
 * now, so the switch never claims a state the system does not have.
 */
export const autostartKey = ["autostart"] as const;

/** Whether the OS starts Boba at login. */
export function useAutostart() {
  return useQuery({
    queryKey: autostartKey,
    queryFn: () => commands.autostartStatus(),
  });
}

/** Set it; the mutation re-reads the status so the switch shows the result. */
export function useSetAutostart() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (enabled: boolean) => {
      await commands.autostartSet(enabled).then(unwrap);
      return commands.autostartStatus();
    },
    onSuccess: (status: AutostartStatus) => queryClient.setQueryData(autostartKey, status),
  });
}
