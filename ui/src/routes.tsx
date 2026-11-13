// The routes the app adds under the signed-in frame: its own screens, then the records frame's pages. A new post leads to the
// composer; the generic form stays reachable for any post at /form/SocialPost/:id.

import type { RouteObject } from "react-router";

const page = (name: "FormView" | "ListView" | "ReportView" | "AppSettingsPage" | "ScreenPage") => () => import("./records").then((m) => ({ Component: m[name] }));

export const routes: RouteObject[] = [
  { index: true, lazy: () => import("./Home").then((m) => ({ Component: m.Home })) },
  { path: "accounts", lazy: () => import("./Accounts").then((m) => ({ Component: m.AccountsRoute })) },
  { path: "compose", lazy: () => import("./Composer").then((m) => ({ Component: m.ComposerRoute })) },
  { path: "compose/:id", lazy: () => import("./Composer").then((m) => ({ Component: m.ComposerRoute })) },
  { path: "calendar", lazy: () => import("./Calendar").then((m) => ({ Component: m.CalendarRoute })) },
  { path: "queue", lazy: () => import("./Queue").then((m) => ({ Component: m.QueueRoute })) },
  { path: "media", lazy: () => import("./Media").then((m) => ({ Component: m.MediaRoute })) },
  { path: "e/SocialPost/:id", lazy: () => import("./Composer").then((m) => ({ Component: m.ComposerRoute })) },
  { path: "form/:entity/:id", lazy: page("FormView") },
  { path: "e/:entity", lazy: page("ListView") },
  { path: "e/:entity/:id", lazy: page("FormView") },
  { path: "r/:name", lazy: page("ReportView") },
  { path: "apps/:app/settings", lazy: page("AppSettingsPage") },
  { path: "x/:app/:page", lazy: page("ScreenPage") },
];
