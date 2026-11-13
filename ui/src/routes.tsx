// The routes the app adds under the signed-in frame: its own screens, then the records frame's pages. A new post leads to the
// composer; the generic form stays reachable for any post at /form/SocialPost/:id.

import type { RouteObject } from "react-router";

const page = (name: "FormView" | "ListView" | "ReportView" | "AppSettingsPage" | "ScreenPage") => () => import("./records").then((m) => ({ Component: m[name] }));

export const routes: RouteObject[] = [
  { index: true, lazy: () => import("./Home").then((m) => ({ Component: m.Home })) },
  { path: "accounts", lazy: () => import("./Accounts").then((m) => ({ Component: m.AccountsRoute })) },
  { path: "form/:entity/:id", lazy: page("FormView") },
  { path: "e/:entity", lazy: page("ListView") },
  { path: "e/:entity/:id", lazy: page("FormView") },
  { path: "r/:name", lazy: page("ReportView") },
  { path: "apps/:app/settings", lazy: page("AppSettingsPage") },
  { path: "x/:app/:page", lazy: page("ScreenPage") },
];
