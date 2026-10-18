import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, events } from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";

/** What the shell shows about the daily search (S5.1/S5.2). */
export interface RunState {
  running: boolean;
  /** The live run's task id: what Cancel hands to `task_cancel` (FX15). */
  runId: string | null;
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
  const [run, setRun] = useState<RunState>({
    running: false,
    runId: null,
    lastRunAt: null,
    newCount: 0,
  });
  const [message, setMessage] = useState<RunMessage | null>(null);
  const [pending, setPending] = useState(false);
  const [cancelling, setCancelling] = useState(false);
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

    // FX15 (review M-2): a run can already be live when this mounts (the tray,
    // the onboarding finish step, a reload mid-run). The live `job_run` id is
    // what Cancel targets, so the shell asks for it instead of waiting for the
    // next progress event.
    void Promise.resolve()
      .then(() => commands.jobsRunning())
      .then(unwrap)
      .then((liveId) => {
        if (!alive || !liveId || finishedRun.current === liveId) return;
        setRun((current) =>
          current.running ? current : { ...current, running: true, runId: liveId },
        );
      })
      .catch(() => {
        // No bridge or no store: the event stream still keeps the chip honest.
      });

    let stop: (() => void) | undefined;
    try {
      void Promise.resolve(
        events.runReportProgressed.listen((event) => {
          if (event.payload.phase === "finished") {
            finishedRun.current = event.payload.run_id;
            setRun((current) => ({ ...current, running: false, runId: null }));
            // The run stopped on its own before the cancel took effect: the
            // label must not keep saying "Cancelling…" (review L-1).
            setCancelling(false);
            refresh();
            return;
          }
          // FX15: the run stopped because the user cancelled it. The store
          // already holds the partial report, so the screen reloads it.
          if (event.payload.phase === "cancelled") {
            finishedRun.current = event.payload.run_id;
            setRun((current) => ({ ...current, running: false, runId: null }));
            setCancelling(false);
            setMessage({ kind: "info", text: t("shell:topbar.cancel.done") });
            refresh();
            return;
          }
          setRun((current) => ({
            ...current,
            running: true,
            runId: event.payload.run_id,
          }));
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

  // Review L-1: if the run is gone, no cancel can still be in flight.
  useEffect(() => {
    if (!run.runId) setCancelling(false);
  }, [run.runId]);

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
        setRun((current) => ({ ...current, running: true, runId: outcome.run_id }));
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

  /**
   * Ask the core to cancel the live run (FX15). The call only *requests* the
   * stop: the run finishes its current Source and the `cancelled` lifecycle
   * event is what moves the screen, so a cancel that lands late never shows a
   * state the core did not reach.
   */
  const cancelRun = useCallback(() => {
    const runId = run.runId;
    if (!runId || cancelling) return;
    setMessage(null);
    setCancelling(true);
    void Promise.resolve()
      .then(() => commands.taskCancel(runId))
      .then(unwrap)
      .then((found) => {
        // No such task: it had already finished (or never started).
        if (!found) {
          setMessage({ kind: "error", text: t("shell:topbar.cancel.gone") });
          setCancelling(false);
        }
        // Accepted: "Cancelling…" stays until the lifecycle event lands, so
        // the label never claims the run is back while it is still stopping.
      })
      .catch((error: unknown) => {
        setMessage({
          kind: "error",
          text: isAppError(error) ? renderAppError(error, t) : t("errors.unknown"),
        });
        setCancelling(false);
      });
  }, [run.runId, cancelling, t]);

  return { run, message, pending, cancelling, runNow, cancelRun };
}
