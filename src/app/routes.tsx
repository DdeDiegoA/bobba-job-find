import { createHashRouter, Navigate, RouterProvider } from "react-router";
import Connections from "@/features/connections";
import Inbox from "@/features/inbox";
import Offer from "@/features/offer";
import Onboarding from "@/features/onboarding";
import { useOnboardingGate } from "@/features/onboarding/wizard/use-onboarding";
import Profile from "@/features/profile";
import Settings from "@/features/settings";
import { OpenToday } from "@/features/settings/notifications/open-today";
import Studio from "@/features/studio";
import Today from "@/features/today";
import Tracker from "@/features/tracker";
import Unlock from "@/features/unlock";
import { NotFound, Shell } from "./shell";

/**
 * The first launch opens setup, not an empty Today (S3.1 AC1, landed by S3.8).
 * The core's `onboarding_status` decides, so a cleared webview storage cannot
 * replay setup; until it answers, nothing renders and nothing bounces.
 */
function FirstRun() {
  const onboarded = useOnboardingGate();
  if (onboarded === null) return null;
  return onboarded ? <Today /> : <Navigate to="/onboarding" replace />;
}

/**
 * Every top-level screen (AC9). `createHashRouter` is the Tauri history
 * strategy (memory/hash, ARCHITECTURE-SPINE §Stack). Each screen is owned by a
 * feature slice; this route table is not edited by them.
 * Onboarding and Unlock are full-window (no shell chrome).
 */
export const routes = [
  { path: "/onboarding/*", element: <Onboarding /> },
  { path: "/unlock", element: <Unlock /> },
  {
    // S5.3: a digest click opens Today from wherever the user is.
    element: (
      <>
        <OpenToday />
        <Shell />
      </>
    ),
    children: [
      { index: true, element: <FirstRun /> },
      { path: "inbox", element: <Inbox /> },
      { path: "offer/:postingId?", element: <Offer /> },
      { path: "studio", element: <Studio /> },
      { path: "tracker", element: <Tracker /> },
      { path: "profile", element: <Profile /> },
      { path: "connections", element: <Connections /> },
      { path: "settings/*", element: <Settings /> },
      { path: "*", element: <NotFound /> },
    ],
  },
];

export const router = createHashRouter(routes);

export function AppRouter() {
  return <RouterProvider router={router} />;
}
