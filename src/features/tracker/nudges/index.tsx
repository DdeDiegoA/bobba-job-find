import { useQueries } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { type ApplicationDetail, commands } from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";
import { detailKey, useBoard, useTransitionStatus } from "../board/use-tracker";
import { type Nudge, nudgesFor } from "./due";

/**
 * Follow-up nudges (S9.5, FR-95). Cards sitting in Applied without a reply get
 * a gentle check-in at ten days and a No Response suggestion at thirty; the
 * suggestion only becomes a move when the user confirms it (AC2). It reads the
 * same board query as the Tracker, so a move updates both.
 */
export default function TrackerNudges() {
  const { t } = useTranslation("nudges");
  const { t: tError } = useTranslation();
  const board = useBoard();
  const move = useTransitionStatus();
  const [confirming, setConfirming] = useState<Nudge | null>(null);
  const [error, setError] = useState<string | null>(null);

  const applied = board.data?.columns.find((column) => column.stage === "applied")?.cards ?? [];
  const details = useQueries({
    queries: applied.map((card) => ({
      queryKey: detailKey(card.id),
      queryFn: () => commands.trackerApplicationDetail(card.id).then(unwrap),
    })),
  });

  const loaded = details
    .map((query) => query.data)
    .filter((detail): detail is ApplicationDetail => detail !== undefined);
  const nudges = nudgesFor(loaded, new Date());

  const commit = (nudge: Nudge, to: "no_response" | "applied") => {
    setError(null);
    setConfirming(null);
    move.mutate(
      {
        application_id: nudge.applicationId,
        from_status: to === "no_response" ? "applied" : "no_response",
        to_status: to,
        source: "user",
        reason: "follow_up_nudge",
      },
      {
        onSuccess: () => {
          if (to === "applied") return;
          toast(t("undo.label", { title: nudge.title }), {
            action: { label: t("undo.action"), onClick: () => commit(nudge, "applied") },
          });
        },
        onError: (failure) =>
          setError(isAppError(failure) ? renderAppError(failure, tError) : t("error.move")),
      },
    );
  };

  if (nudges.length === 0 && error === null) return null;

  return (
    <section
      className="flex flex-col gap-2 rounded-2xl border border-border bg-surface p-3"
      aria-labelledby="nudges-title"
    >
      <h2 id="nudges-title" className="font-display text-sm text-fg">
        {t("title")}
      </h2>
      {error ? <p className="text-sm text-danger">{error}</p> : null}
      <ul className="flex flex-col gap-2">
        {nudges.map((nudge) => (
          <li key={nudge.applicationId} className="flex flex-col gap-2">
            <p className="text-sm text-fg">
              {nudge.kind === "no_response"
                ? t("no_response", {
                    title: nudge.title,
                    company: nudge.company,
                    days: nudge.days,
                  })
                : t("check_in", {
                    title: nudge.title,
                    company: nudge.company,
                    days: nudge.days,
                  })}
            </p>
            {nudge.kind === "no_response" ? (
              <Button size="sm" variant="ghost" onClick={() => setConfirming(nudge)}>
                {t("action.no_response")}
              </Button>
            ) : null}
          </li>
        ))}
      </ul>

      {confirming ? (
        <div className="flex flex-col gap-2 rounded-xl border border-border bg-surface-muted p-2">
          <p className="text-sm text-fg">{t("confirm.no_response", { title: confirming.title })}</p>
          <div className="flex gap-2">
            <Button size="sm" onClick={() => commit(confirming, "no_response")}>
              {t("confirm.yes")}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setConfirming(null)}>
              {t("confirm.no")}
            </Button>
          </div>
        </div>
      ) : null}
    </section>
  );
}
