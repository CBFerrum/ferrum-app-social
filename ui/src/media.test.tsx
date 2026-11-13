// @vitest-environment jsdom
// The media library as it is drawn: assets in a grid, a delete the server refuses (its words shown), stock search through the
// sources the settings name with the credit shown and "take" storing it, no source set, and a picture from an address.

import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { MediaRoute } from "./Media";
import { sourcesOf } from "./media/Library";
import { byText, draw, fakeServer, settle, until } from "./testkit";

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const assets = { rows: [
  { id: "m1", version: 1, asset_name: "spring.png", kind: "image", file: "/files/f1", alt: "", bytes: 20480, source: "Upload", brand: "b1" },
  { id: "m2", version: 1, asset_name: "clip.mp4", kind: "video", file: "/files/f2", alt: "", bytes: 1048576, source: "Stock", credit: "Ann / Stockpics", brand: "b1" },
] };
const base = (extra: Parameters<typeof fakeServer>[0] = [], sources = "stockpics") =>
  fakeServer([
    ...extra,
    ["GET", "/api/v1/SocialSettings", { rows: [{ id: "s", media_sources: sources }] }],
    ["GET", "/api/v1/SocialMediaAsset", assets],
    ["GET", "/api/v1/SocialBrand", { rows: [{ id: "b1", brand_name: "Northwind" }] }],
    ["GET", "/api/tags", { tags: [] }],
  ]);
const click = async (el: HTMLElement | undefined | null) => {
  expect(el).toBeTruthy();
  await act(async () => el!.click());
  await settle();
};
const type = async (el: HTMLElement | null, value: string) => {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(el, value);
    el!.dispatchEvent(new Event("input", { bubbles: true }));
  });
};

describe("the media library", () => {
  it("lists assets with their kind, source and credit, and says which lack a description", async () => {
    base();
    const h = await draw(<MediaRoute />, "/media", "/media");
    expect(h.textContent).toContain("spring.png");
    expect(h.textContent).toContain("Ann / Stockpics");
    expect(h.textContent).toContain("No description");
  });

  it("a delete the server refuses shows its words and leaves the asset", async () => {
    base([["DELETE", "/api/v1/SocialMediaAsset/m1", { error: "conflict", message: "spring.png is used by a post" }, 409]]);
    const host = await draw(<MediaRoute />, "/media", "/media");
    await click(host.querySelector<HTMLElement>('button[aria-label="Delete spring.png"]'));
    await click([...document.querySelectorAll<HTMLElement>("dialog button")].find((b) => b.textContent === "Delete"));
    await until(() => document.body.textContent!.includes("used by a post"));
    expect(host.textContent).toContain("spring.png is used by a post");
    expect(host.textContent).toContain("spring.png");
  });

  it("finds stock pictures through the source the settings name, shows the credit, and takes one", async () => {
    const fake = base([
      ["POST", "/api/v1/SocialBrand/b1/actions/media_search", { source: "stockpics", page: 1, items: [{ id: "px-1", preview: "https://cdn.example/p.jpg", url: "https://cdn.example/full.jpg", credit: "Bo Lin", credit_link: "https://stock.example/bo", licence: "Free to use", width: 800, height: 600, alt: "A hill", kind: "image" }] }],
      ["POST", "/api/v1/SocialBrand/b1/actions/media_take", { entity: "SocialMediaAsset", id: "m3", file: "/files/f3" }],
    ]);
    const host = await draw(<MediaRoute />, "/media", "/media");
    await click([...host.querySelectorAll<HTMLElement>('[role=tab]')].find((b) => b.textContent === "Stock and GIFs"));
    await type(host.querySelector("#st-q"), "hill");
    await until(() => host.textContent!.includes("Bo Lin"));
    expect(host.textContent).toContain("Free to use");
    await click(byText(host, "button", "Take it"));
    const sent = fake.seen.find((s) => s.line.endsWith("/media_take"))?.body as Record<string, unknown>;
    expect(sent).toMatchObject({ id: "px-1", source: "stockpics", url: "https://cdn.example/full.jpg", credit: "Bo Lin", licence: "Free to use", width: 800 });
    expect(fake.seen.find((s) => s.line.endsWith("/media_search"))?.body).toMatchObject({ query: "hill", source: "stockpics", page: 1 });
  });

  it("with no source named, says so in plain words", async () => {
    base([], "");
    const host = await draw(<MediaRoute />, "/media", "/media");
    await click([...host.querySelectorAll<HTMLElement>('[role=tab]')].find((b) => b.textContent === "Stock and GIFs"));
    expect(host.textContent).toContain("No picture source is set");
  });

  it("adds a picture from an address, and shows the server's refusal", async () => {
    base([["POST", "/api/v1/SocialBrand/b1/actions/media_from_address", { error: "invalid", message: "no connection is set for fetching media from an address" }, 422]]);
    const host = await draw(<MediaRoute />, "/media", "/media");
    await click([...host.querySelectorAll<HTMLElement>('[role=tab]')].find((b) => b.textContent === "From an address"));
    await type(host.querySelector("#ad-url"), "https://example.org/a.png");
    await click(byText(host, "button", "Fetch it"));
    expect(host.textContent).toContain("no connection is set for fetching media from an address");
  });
});

describe("the sources", () => {
  it("are the connection names, one a line, the first the default", () => {
    expect(sourcesOf({ id: "s", media_sources: "stockpics\n giphy ,tenor" })).toEqual(["stockpics", "giphy", "tenor"]);
    expect(sourcesOf(undefined)).toEqual([]);
  });
});
