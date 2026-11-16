// Social's own interface at /social/: the composer, the calendar, posting times and accounts, posts by day, what waits for
// approval, the inbox and what posts did, with every other record in the records frame's lists and forms.

import { useEffect } from "react";
import { Link, useRouteError } from "react-router";
import { shell, t } from "@ferrum/client";
import { EmptyState, ErrorState, FormSkeleton } from "@ferrum/kit";
import { mountApp, RequireAuth, setFrameMode } from "@ferrum/records/frame";
import { routes } from "./routes";
import "./styles.css";

function RouteError() {
  return <ErrorState error={useRouteError()} onRetry={() => window.location.reload()} />;
}

function NotFound() {
  return (
    <EmptyState title={t("Page not found")}>
      <Link to="/">{t("Go to the posts")}</Link>
    </EmptyState>
  );
}

function Started() {
  useEffect(() => shell().ready(), []);
  return null;
}

setFrameMode("app", ["social"]);

mountApp({
  route: "social",
  catalogs: { ar: () => import("./locales/ar.json"), hi: () => import("./locales/hi.json") },
  routes: [
    {
      path: "/",
      element: (
        <>
          <Started />
          <RequireAuth />
        </>
      ),
      errorElement: <RouteError />,
      // Drawn while the router starts: the kit's page skeleton, so no "No HydrateFallback" warning and no blank page.
      HydrateFallback: FormSkeleton,
      children: [
        ...routes,
        { path: "*", element: <NotFound /> },
      ],
    },
  ],
});
