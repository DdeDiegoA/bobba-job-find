import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  commands,
  type EditInput,
  type ExportedFile,
  type StudioKind,
  type StudioView,
} from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/** The studio is one query per application: both drafts and their checks (AC1). */
export const studioKey = (applicationId: string) => ["studio", applicationId] as const;

/** Every read and write returns the whole view, which replaces the cached one. */
function useStudioMutation<TArgs>(
  applicationId: string,
  mutationFn: (args: TArgs) => Promise<StudioView>,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn,
    onSuccess: (view: StudioView) => queryClient.setQueryData(studioKey(applicationId), view),
  });
}

/**
 * The applications the studio can open when the route names none: the same
 * board the Tracker shows (S9.1), so the screen never invents its own list.
 */
export function useApplications() {
  return useQuery({
    queryKey: ["studio", "applications"] as const,
    queryFn: () => commands.trackerBoard().then(unwrap),
  });
}

export function useStudioView(applicationId: string) {
  return useQuery({
    queryKey: studioKey(applicationId),
    enabled: applicationId.length > 0,
    queryFn: () => commands.studioView(applicationId).then(unwrap),
  });
}

/** Save one inline edit: the core stores a new version and re-runs the checks. */
export function useStudioEdit(applicationId: string) {
  return useStudioMutation<EditInput>(applicationId, (input) =>
    commands.studioEdit(input).then(unwrap),
  );
}

/** Render both drafts: fit, render, verify (S7.5/S7.6). */
export function useStudioRender(applicationId: string) {
  return useStudioMutation<void>(applicationId, () =>
    commands.studioRender(applicationId).then(unwrap),
  );
}

/** Draft, review and revise the application's documents (architecture §5.5). */
export function useStudioGenerate(applicationId: string) {
  return useStudioMutation<void>(applicationId, () =>
    commands.studioGenerate(applicationId).then(unwrap),
  );
}

/** Regenerate one section and copy it into the version the user is reading. */
export function useStudioRegenerate(applicationId: string) {
  return useStudioMutation<{
    kind: StudioKind;
    section: number | null;
    into: string;
  }>(applicationId, ({ kind, section, into }) =>
    commands.studioRegenerate(applicationId, kind, section, into).then(unwrap),
  );
}

/** Export the newest draft of one kind as a DOCX (FR-78). */
export function useStudioExport(applicationId: string) {
  return useMutation<ExportedFile, unknown, StudioKind>({
    mutationFn: (kind: StudioKind) => commands.studioExportDocx(applicationId, kind).then(unwrap),
  });
}
