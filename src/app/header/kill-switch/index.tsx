import { OctagonX } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { commands, events, type KillSwitchView } from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";

/**
 * Header Kill Switch (S9.8, FR-86, EXPERIENCE §9.9).
 *
 * One control, three ways in: the tray, Cmd/Ctrl+Shift+K and this header chip.
 * It stops every automatic send immediately, parks the Outbox items for review
 * and stays on — the quiet chip — until the user turns it off here.
 *
 * Deliberately no query cache: the topbar renders in shells that have none, and
 * the state is one small command plus the `kill_switch.changed` event that the
 * tray and the shortcut also emit.
 */
export const KILL_SWITCH_SHORTCUT = "Meta+Shift+K Control+Shift+K";

export default function KillSwitch() {
  const { t } = useTranslation("kill_switch");
  const { t: tError } = useTranslation();
  const [status, setStatus] = useState<KillSwitchView | null>(null);
  const [pending, setPending] = useState(false);

  useEffect(() => {
    let alive = true;
    // `invoke` throws synchronously without a Tauri bridge (a plain-browser
    // preview); the Promise chain keeps that from escaping the effect.
    void Promise.resolve()
      .then(() => commands.killSwitchStatus())
      .then(unwrap)
      .then((view) => {
        if (alive) setStatus(view);
      })
      .catch(() => {});
    let stop: (() => void) | undefined;
    try {
      void Promise.resolve(
        events.killSwitchChanged.listen((event) => {
          setStatus({ engaged: event.payload.engaged, since: event.payload.since });
        }),
      )
        .then((unlisten) => {
          if (alive) stop = unlisten;
          else unlisten();
        })
        .catch(() => {});
    } catch {
      // Same as above: the panel works without the event stream.
    }
    return () => {
      alive = false;
      stop?.();
    };
  }, []);

  const change = (engaged: boolean) => {
    setPending(true);
    void Promise.resolve()
      .then(() => commands.killSwitchSet(engaged))
      .then(unwrap)
      .then((result) => setStatus(result.view))
      .catch((error: unknown) => {
        toast.error(isAppError(error) ? renderAppError(error, tError) : t("error"));
      })
      .finally(() => setPending(false));
  };

  if (status?.engaged) {
    return (
      <div className="flex items-center gap-1">
        <span
          role="status"
          title={t("engaged.hint")}
          className="flex h-7 items-center gap-1.5 rounded-full border border-border-strong bg-surface px-2.5 text-xs text-fg-muted"
        >
          <OctagonX aria-hidden className="size-3.5" />
          {t("engaged")}
        </span>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={pending}
          onClick={() => change(false)}
        >
          {t("turnOff")}
        </Button>
      </div>
    );
  }

  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      title={t("engage.hint")}
      aria-keyshortcuts={KILL_SWITCH_SHORTCUT}
      disabled={pending}
      onClick={() => change(true)}
    >
      <OctagonX aria-hidden className="size-4" />
      {t("engage")}
    </Button>
  );
}
