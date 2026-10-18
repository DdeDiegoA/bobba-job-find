import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, events } from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";

/** What the shell shows about the daily search (S5.1/S5.2). */
export interface RunState {
  running: boolean;
  lastRunAt: string | null;
  newCount: number;
}

/** The reason Run now could not start, or that it joined a live run (FX4). */
export interface RunMessage {
  kind: "error" | "info";
  text: string;
}

/**
 * The shell's run state and the topbar's Run now (S5.1, S5.2, FX4).
 *
 * The command starts or joins the run; `run_report:progressed` is what keeps
 * the button and the last-run line honest, so a run started from the tray or
 * the onboarding finish step shows up here too. Nothing is cached: the shell
 * renders outside every feature's query scope (same reason as the Kill Switch).
 */
export function useRunNow() {
  const { t } = useTranslation();
  const [run, setRun] = useState<RunState>({ running: false, lastRunAt: null, newCount: 0 });
  const [message, setMessage] = useState<RunMessage | null>(null);
  const [pending, setPending] = useState(false);
  // A short run can end before `jobs_run_now` answers (the command spawns it).
  // Remember its id so the command's answer never re-opens the running state.
  const finishedRun = useRef<string | null>(null);

  useEffect(() => {
    let alive = true;

    const refresh = () => {
      // `invoke` throws synchronously without a Tauri bridge (a plain-browser
      // preview); the Promise chain keeps that from escaping the effect.
      void Promise.resolve()
        .then(() => commands.runReportLatest())
        .then(unwrap)
        .then((report) => {
          if (!alive || !report) return;
          setRun((current) => ({
            ...current,
            lastRunAt: report.finished_at,
            newCount: report.new,
          }));
        })
        .catch((error: unknown) => {
          // The read failed: say why instead of leaving "No runs yet" up as if
          // there had been no runs (FX4 review Low 2).
          if (!alive) return;
          setMessage({
            kind: "error",
            text: isAppError(error) ? renderAppError(error, t) : t("errors.unknown"),
          });
        });
    };

    refresh();

    let stop: (() => void) | undefined;
    try {
      void Promise.resolve(
        events.runReportProgressed.listen((event) => {
          if (event.payload.phase === "finished") {
            finishedRun.current = event.payload.run_id;
            setRun((current) => ({ ...current, running: false }));
            refresh();
            return;
          }
          setRun((current) => ({ ...current, running: true }));
        }),
      )
        .then((unlisten) => {
          if (alive) stop = unlisten;
          else unlisten();
        })
        .catch(() => {});
    } catch {
      // Same as above: the button still works without the event stream.
    }

    return () => {
      alive = false;
      stop?.();
    };
  }, [t]);

  const runNow = useCallback(() => {
    setMessage(null);
    setPending(true);
    void Promise.resolve()
      .then(() => commands.jobsRunNow())
      .then(unwrap)
      .then((outcome) => {
        // An empty id means the slot was already recorded, and a run that
        // already finished has nothing left to follow: either way the button
        // must not stay disabled waiting for an event that will not come.
        if (outcome.run_id === "" || finishedRun.current === outcome.run_id) return;
        setRun((current) => ({ ...current, running: true }));
        if (outcome.joined) {
          setMessage({ kind: "info", text: t("shell:topbar.runNow.busy") });
        }
      })
      .catch((error: unknown) => {
        setMessage({
          kind: "error",
          text: isAppError(error) ? renderAppError(error, t) : t("errors.unknown"),
        });
      })
      .finally(() => setPending(false));
  }, [t]);

  return { run, message, pending, runNow };
}
