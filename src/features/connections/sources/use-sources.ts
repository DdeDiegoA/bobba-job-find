import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { commands, type RssTestView, type SourceTileView } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/** The Connections screen is one query: every Source with its health (AC1). */
export const sourceCatalogueKey = ["sources", "catalogue"] as const;

/** Every Source, built-in and custom, with its health. */
export function useSourceCatalogue() {
  return useQuery({
    queryKey: sourceCatalogueKey,
    queryFn: () => commands.sourcesCatalogue().then(unwrap),
  });
}

/**
 * The add-feed probe (AC2). Nothing is persisted: the preview the user accepts
 * is what [`useAddRss`] saves afterwards.
 */
export function useTestRss() {
  return useMutation({
    mutationFn: ({ url, name }: { url: string; name: string | null }) =>
      commands.sourcesTestRss(url, name).then(unwrap),
  });
}

/** Save the feed the user previewed; the catalogue reloads with its tile. */
export function useAddRss() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ url, name }: { url: string; name: string | null }) =>
      commands.sourcesAddRss(url, name).then(unwrap),
    onSuccess: (tile: SourceTileView) => {
      queryClient.setQueryData<SourceTileView[]>(sourceCatalogueKey, (current = []) =>
        [...current.filter((row) => row.id !== tile.id), tile].sort((a, b) =>
          a.name.toLowerCase().localeCompare(b.name.toLowerCase()),
        ),
      );
    },
  });
}

/** Remove a custom feed; the catalogue reloads without it. */
export function useRemoveRss() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => commands.sourcesRemoveRss(id).then(unwrap),
    onSuccess: (_removed: boolean, id: string) => {
      queryClient.setQueryData<SourceTileView[]>(sourceCatalogueKey, (current = []) =>
        current.filter((row) => row.id !== id),
      );
    },
  });
}

/** The probe result the screen keeps while the form is open (AC2). */
export type RssProbe = RssTestView;
