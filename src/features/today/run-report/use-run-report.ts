import { useMutation, useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { commands, events, type RunReportView } from "@/lib/ipc/bindings";
import { unwrap } from "@/lib/ipc/errors";

/** The newest run's report is one cached query, shared by the panel and Today. */
export const runReportKey = ["run-report", "latest"] as const;

/** The newest run's report (AC2), `null` before the first run. */
export function useLatestRunReport() {
  return useQuery({
    queryKey: runReportKey,
    queryFn: () => commands.runReportLatest().then(unwrap),
  });
}

/**
 * Retry the Sources the newest report could not reach (AC3). The command
 * returns as soon as the retry starts; `run_report:progressed` is what tells
 * the screen to reload, so nothing here waits on the network.
 */
export function useRetryRunReport() {
  return useMutation({
    mutationFn: () => commands.runReportRetry().then(unwrap),
  });
}

/** A run in flight, as the topbar shows it. */
export interface RunProgress {
  runId: string;
  /** 0 while the run is starting, 1..=total as Sources finish. */
  done: number;
  total: number;
  sourceName: string | null;
}

/**
 * The live progress of a running search (AC1), from `run_report:progressed`.
 *
 * `onFinished` is called when the run ends — the screen that shows the report
 * reloads it there, which is the only moment it reads the run again: the UI
 * never polls. The topbar passes nothing, so it needs no query context.
 */
export function useRunProgress(onFinished?: () => void): RunProgress | null {
  const [progress, setProgress] = useState<RunProgress | null>(null);

  useEffect(() => {
    try {
      const unlisten = events.runReportProgressed.listen((event) => {
        const payload = event.payload;
        if (payload.phase === "finished") {
          setProgress(null);
          onFinished?.();
          return;
        }
        setProgress({
          runId: payload.run_id,
          done: payload.done,
          total: payload.total,
          sourceName: payload.phase === "source" ? payload.source_name : null,
        });
      });
      return () => {
        void unlisten.then((stop) => stop()).catch(() => {});
      };
    } catch {
      // No Tauri bridge (a plain-browser preview): runs still work, the topbar
      // just has nothing live to show.
      return;
    }
  }, [onFinished]);

  return progress;
}

/** A `RunReportView`'s failed Sources, for the banner (AC3). */
export function failedSources(report: RunReportView) {
  return report.sources.filter((source) => source.outcome === "failed");
}

/** A run's duration as a short human string: `0:42`, `2:07`, `1:02:07`. */
export function formatDuration(durationMs: number): string {
  const totalSeconds = Math.max(0, Math.round(durationMs / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = String(totalSeconds % 60).padStart(2, "0");
  if (hours > 0) return `${hours}:${String(minutes).padStart(2, "0")}:${seconds}`;
  return `${minutes}:${seconds}`;
}
