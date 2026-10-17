import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import { Button } from "@/components/ui/button";
import { type CapabilityId, useCapability } from "@/lib/capabilities";
import { cn } from "@/lib/utils";

/** Where the user connects (or changes) an Agent CLI. */
const AGENT_SETUP_PATH = "/onboarding";

/**
 * One notice per reason: Limited mode asks for an agent (AC2); a CLI without
 * verified `tool_lockdown` asks for another CLI, never an override (AC3,
 * EXPERIENCE §9.15).
 */
const NOTICE: Record<"no_agent" | "cli_unsupported", { title: string; body: string; cta: string }> =
  {
    no_agent: {
      title: "gated:notice.limitedTitle",
      body: "gated:notice.limitedBody",
      cta: "gated:notice.connect",
    },
    cli_unsupported: {
      title: "gated:notice.unsupportedTitle",
      body: "gated:notice.unsupportedBody",
      cta: "gated:notice.changeCli",
    },
  };

/**
 * The shared wrapper for an AI-only action (AC4).
 *
 * Available: renders the action untouched. Off: renders the action **disabled**
 * (never hidden) with the reason and its call to action, so every feature slice
 * shows the same state without re-implementing it (DESIGN AIUnavailableState).
 * While the snapshot loads — or when it cannot be read — the action is
 * disabled without a notice: no control that would fail later.
 */
export function Gated({
  capability,
  children,
  className,
}: {
  capability: CapabilityId;
  children: ReactNode;
  className?: string;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const access = useCapability(capability);

  if (access.available) return <>{children}</>;

  const notice = access.reason ? NOTICE[access.reason] : null;

  return (
    <div className={cn("flex min-w-0 flex-col gap-2", className)} aria-disabled>
      {notice ? (
        <div
          className="flex flex-col items-start gap-1 rounded-xl bg-surface-muted p-3 text-sm"
          role="status"
        >
          <p className="font-medium text-fg">{t(notice.title)}</p>
          <p className="text-fg-muted">{t(notice.body)}</p>
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() => navigate(AGENT_SETUP_PATH)}
          >
            {t(notice.cta)}
          </Button>
        </div>
      ) : null}
      <fieldset disabled className="m-0 min-w-0 border-0 p-0">
        {children}
      </fieldset>
    </div>
  );
}
