import { createHashRouter, RouterProvider } from "react-router";
import Connections from "@/features/connections";
import Inbox from "@/features/inbox";
import Offer from "@/features/offer";
import Onboarding from "@/features/onboarding";
import Profile from "@/features/profile";
import Settings from "@/features/settings";
import Studio from "@/features/studio";
import Today from "@/features/today";
import Tracker from "@/features/tracker";
import Unlock from "@/features/unlock";
import { NotFound, Shell } from "./shell";

/**
 * Every top-level screen (AC9). `createHashRouter` is the Tauri history
 * strategy (memory/hash, ARCHITECTURE-SPINE §Stack). Placeholder pages are
 * replaced by their owning slice; this route table is not edited by them.
 * Onboarding and Unlock are full-window (no shell chrome).
 */
export const routes = [
  { path: "/onboarding/*", element: <Onboarding /> },
  { path: "/unlock", element: <Unlock /> },
  {
    element: <Shell />,
    children: [
      { index: true, element: <Today /> },
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
