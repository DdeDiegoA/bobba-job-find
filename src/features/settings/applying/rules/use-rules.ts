import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { commands, type RuleInput } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/** The rules and the global Auto settings are one read (S9.6, AC1). */
export const rulesKey = ["apply", "rules"] as const;

/** Everything Settings → Applying shows. */
export function useRules() {
  return useQuery({
    queryKey: rulesKey,
    queryFn: () => commands.applyRulesSnapshot().then(unwrap),
  });
}

/** Create or edit a rule; the core validates every bound (AD-11). */
export function useSaveRule() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: RuleInput) => commands.applyRuleSave(input).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: rulesKey }),
  });
}

/** Remove a rule. */
export function useDeleteRule() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (ruleId: string) => commands.applyRuleDelete(ruleId).then(unwrap),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: rulesKey }),
  });
}

/** Turn Auto on (confirmed first, AC3) or off. */
export function useSetAuto() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ enabled, confirmed }: { enabled: boolean; confirmed: boolean }) =>
      commands.applyRulesSetEnabled(enabled, confirmed).then(unwrap),
    onSuccess: (snapshot) => queryClient.setQueryData(rulesKey, snapshot),
  });
}

/** FR-108: turn Auto off and restore the default caps. */
export function useResetRules() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => commands.applyRulesReset().then(unwrap),
    onSuccess: (snapshot) => queryClient.setQueryData(rulesKey, snapshot),
  });
}

/** The cap across every rule (FR-83). */
export function useSetGlobalCap() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (cap: number) => commands.applyRulesSetGlobalCap(cap).then(unwrap),
    onSuccess: (snapshot) => queryClient.setQueryData(rulesKey, snapshot),
  });
}

/** Why Auto would or would not send one application (AC2). */
export function useExplain() {
  return useMutation({
    mutationFn: ({
      applicationId,
      channel,
      localMinute,
      utcOffsetMinutes,
    }: {
      applicationId: string;
      channel: string;
      localMinute: number | null;
      utcOffsetMinutes: number | null;
    }) =>
      commands
        .applyRulesExplain(applicationId, channel, localMinute, utcOffsetMinutes)
        .then(unwrap),
  });
}
