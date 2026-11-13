// What the screens' tests share: a page drawn in jsdom over a fake server (a list of answers by method and path), and a wait
// for what it asks to arrive. Answers are the server's own shapes, copied from its code and tests.

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { createMemoryRouter, RouterProvider } from "react-router";
import { vi } from "vitest";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

export type Answer = unknown | ((url: URL, body: unknown) => unknown);
export interface Fake {
  /** `[method, path prefix, answer, status]`: the first that fits answers; an answer may be a function of the request. */
  routes: [string, string, Answer, number?][];
  /** What was asked, in order: `METHOD path?query` and its body. */
  seen: { line: string; body: unknown }[];
}

/** Puts a fake server in place of the network. */
export function fakeServer(routes: Fake["routes"]): Fake {
  const fake: Fake = { routes, seen: [] };
  // jsdom has no modal dialog: an open one is enough for a test to read and press what is in it.
  const dialog = HTMLDialogElement.prototype as { showModal?: () => void; close?: () => void };
  dialog.showModal = function (this: HTMLDialogElement) { this.setAttribute("open", ""); };
  dialog.close = function (this: HTMLDialogElement) { this.removeAttribute("open"); };
  (globalThis as { matchMedia?: unknown }).matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), "http://localhost");
      const method = init?.method ?? "GET";
      const body = typeof init?.body === "string" ? (JSON.parse(init.body) as unknown) : undefined;
      fake.seen.push({ line: `${method} ${url.pathname}${url.search}`, body });
      const hit = fake.routes.find(([m, p]) => m === method && url.pathname.startsWith(p));
      const [, , answer, status = 200] = hit ?? ["", "", { error: "not_found", message: "Not found." }, 404];
      const out = typeof answer === "function" ? (answer as (u: URL, b: unknown) => unknown)(url, body) : answer;
      return new Response(JSON.stringify(out), { status, headers: { "content-type": "application/json" } });
    }),
  );
  return fake;
}

/** Draws `ui` at `path` (routes by pattern, in a data router: the composer's guard against leaving needs one) and waits for the first answers. */
export async function draw(ui: ReactNode, pattern: string, path: string) {
  const host = document.createElement("div");
  document.body.append(host);
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } });
  const router = createMemoryRouter([{ path: pattern, element: ui }], { initialEntries: [path] });
  await act(async () => {
    createRoot(host).render(
      <QueryClientProvider client={qc}>
        <RouterProvider router={router} />
      </QueryClientProvider>,
    );
  });
  await settle();
  return host;
}

/** Lets what the page asked for come back and be drawn. */
export const settle = async (ms = 60) => {
  await act(async () => {
    await new Promise((r) => setTimeout(r, ms));
  });
};

/** A button or link by its words. */
export const byText = (host: HTMLElement, selector: string, words: string) => [...host.querySelectorAll<HTMLElement>(selector)].find((e) => (e.textContent ?? "").trim() === words);

/** Waits, a little at a time, until `ok()` holds (3 seconds at most), so a test does not count on how long the page takes. */
export const until = async (ok: () => boolean) => {
  for (let i = 0; i < 60 && !ok(); i++) await settle(50);
};
