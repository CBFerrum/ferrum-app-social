// @vitest-environment jsdom
// The library: tabs of what posts are made from, a template or a set starting a post (the composer fills from it), a signature's
// default mark, and a delete the server refuses.

import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { LibraryRoute } from "./Library";
import { ComposerRoute } from "./Composer";
import { draw, fakeServer, settle, until } from "./testkit";

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const common: Parameters<typeof fakeServer>[0] = [
  ["GET", "/api/v1/SocialBrand", { rows: [{ id: "b1", brand_name: "Northwind" }] }],
  ["GET", "/api/v1/SocialAccount", { rows: [{ id: "x", account_name: "North Short", brand: "b1", state: "Connected", active: true }] }],
];
const click = async (el: HTMLElement | undefined | null) => {
  expect(el).toBeTruthy();
  await act(async () => el!.click());
  await settle();
};
const tab = (host: HTMLElement, name: string) => [...host.querySelectorAll<HTMLElement>("[role=tab]")].find((b) => b.textContent === name);

describe("the library", () => {
  it("lists templates with a way to use each, and signatures with their default", async () => {
    fakeServer([
      ["GET", "/api/v1/SocialTemplate", { rows: [{ id: "t1", template_name: "Weekly tip", brand: "b1" }] }],
      ["GET", "/api/v1/SocialSignature", { rows: [{ id: "s1", signature_name: "Sign-off", brand: "b1", is_default: true }] }],
      ...common,
    ]);
    const host = await draw(<LibraryRoute />, "/library", "/library");
    await until(() => host.textContent!.includes("Weekly tip"));
    expect(host.querySelector('a[href="/compose?template=t1"]')).toBeTruthy();
    await click(tab(host, "Signatures"));
    await until(() => host.textContent!.includes("Sign-off"));
    expect(host.textContent).toContain("Default");
  });

  it("a delete the server refuses shows its words", async () => {
    fakeServer([
      ["DELETE", "/api/v1/SocialSnippet/n1", { error: "conflict", message: "the snippet is in use" }, 409],
      ["GET", "/api/v1/SocialSnippet", { rows: [{ id: "n1", snippet_name: "Hashtags", kind: "Hashtags" }] }],
      ...common,
    ]);
    const host = await draw(<LibraryRoute />, "/library", "/library");
    await click(tab(host, "Snippets"));
    await until(() => host.textContent!.includes("Hashtags"));
    await click(host.querySelector<HTMLElement>('button[aria-label="Delete Hashtags"]'));
    await click([...document.querySelectorAll<HTMLElement>("dialog button")].find((b) => b.textContent === "Delete"));
    await until(() => host.textContent!.includes("the snippet is in use"));
  });

  it("the composer fills from a template it is started with", async () => {
    fakeServer([
      ["GET", "/api/v1/SocialTemplate/t1", { id: "t1", template_name: "Weekly tip", headline: "Tip of the week", text: "Here is the tip", targets: [{ account: "x" }], items: [], media: [] }],
      ["POST", "/api/v1/SocialAccount/x/actions/rules", { rules: { text: { count: "characters" }, media: { kinds: [], required: false, mixes: [], other: {}, alt: true, alt_required: false, thumbnail: false }, kinds: [], follow_ups: { as: "none", media: false }, settings: [], operations: {}, first_comment: false, link_preview: false, scheduling: true } }],
      ["POST", "/api/v1/SocialPost/_calc", { checks: { copies: [] } }],
      ...common,
      ["GET", "/api/v1/Social", { rows: [] }],
      ["GET", "/api/tags", { tags: [] }],
    ]);
    const host = await draw(<ComposerRoute />, "/compose", "/compose?template=t1");
    await until(() => (host.querySelector("#post-headline") as HTMLInputElement | null)?.value === "Tip of the week");
    expect((host.querySelector("#original-item-0") as HTMLTextAreaElement).value).toBe("Here is the tip");
  });
});
