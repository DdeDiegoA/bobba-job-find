import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { commands, type NotificationSettings } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/**
 * The notification preferences as the core holds them (S5.3): one query, so
 * the page and the digest the run shows never disagree about what the user
 * chose. The core clamps what it stores; this layer never invents values.
 */
export const notificationsKey = ["notifications"] as const;

/** The stored notification settings (FR-107). */
export function useNotifications() {
  return useQuery({
    queryKey: notificationsKey,
    queryFn: () => commands.notificationsSnapshot().then(unwrap),
  });
}

/** Save an edit; the core returns the settings it actually stored. */
export function useSaveNotifications() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (settings: NotificationSettings) =>
      commands.notificationsSave(settings).then(unwrap),
    onSuccess: (saved: NotificationSettings) => queryClient.setQueryData(notificationsKey, saved),
  });
}
