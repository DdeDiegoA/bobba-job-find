import { useEffect } from "react";
import { useNavigate } from "react-router";
import { events } from "@/lib/ipc/bindings";

/**
 * A digest notification was clicked, so open Today (S5.3 AC1, FR-45).
 *
 * Tauri's notification plugin reports no click on desktop; the core turns the
 * focus a click brings to the window into this event. The component renders
 * nothing and is mounted once, beside the shell.
 */
export function OpenToday() {
  const navigate = useNavigate();

  useEffect(() => {
    let alive = true;
    let stop: (() => void) | undefined;
    // No Tauri bridge (a plain-browser preview) is not worth an error: the app
    // simply never receives the click.
    try {
      void Promise.resolve(
        events.notificationOpened.listen((event) => {
          if (event.payload.target === "today") navigate("/");
        }),
      )
        .then((unlisten) => {
          if (alive) stop = unlisten;
          else unlisten();
        })
        .catch(() => {});
    } catch {
      // Same as above.
    }
    return () => {
      alive = false;
      stop?.();
    };
  }, [navigate]);

  return null;
}
