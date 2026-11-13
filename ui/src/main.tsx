// Social's own interface at /social/: the composer, the calendar, posting times and accounts, posts by day, what waits for
// approval, the inbox and what posts did, with every other record in the records frame's lists and forms.

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, Link, RouterProvider, useRouteError } from "react-router";
import { addCatalogs, routeBase, shell, t, wordsReady } from "@ferrum/client";
import { ConfirmHost, EmptyState, ErrorState, FormSkeleton, Toaster } from "@ferrum/kit";
import { RequireAuth, setFrameMode } from "@ferrum/records/frame";
import { routes } from "./routes";
import "./styles.css";

// Its own words in the page's language (src/locales), fetched as the page starts; it is drawn once they are in.
void addCatalogs({ ar: () => import("./locales/ar.json"), hi: () => import("./locales/hi.json") });

const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false, staleTime: 5_000 } } });

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

const router = createBrowserRouter(
  [
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
  { basename: routeBase("social") },
);

// Drawn once the page's words are in (its language's catalogs, fetched as the page starts).
void wordsReady().then(() =>
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <RouterProvider router={router} />
        <Toaster />
        <ConfirmHost />
      </QueryClientProvider>
    </StrictMode>,
  ),
);
