// @vitest-environment jsdom
// Writing help in the composer: a rewrite comes back as a proposal and changes nothing until taken; an adapt shows before and
// after for each account; the server's sentence when no connection is set (with the settings for a manager); and the panel is
// absent for a person without the role.

import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { ComposerRoute } from "./Composer";
import { mediaIndex, emptyDraft } from "./draft";
import { byText, draw, fakeServer, settle, until } from "./testkit";

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const rules = { rules: { name: "Shortline", text: { count: "characters", max: 100 }, media: { kinds: [], required: false, mixes: [], other: {}, alt: true, alt_required: false, thumbnail: false }, kinds: [], follow_ups: { as: "thread", media: false }, settings: [], operations: {}, first_comment: false, link_preview: false, scheduling: true } };
const calc = () => ({ checks: { copies: [{ account: "x", account_name: "North Short", network: "Shortline", kind: null, follow_ups: "thread", items: [{ text: "Spring", link: null, delay_minutes: 0, length: 6, max: 100, media: [] }], first_comment: null, problems: [] }] } });
const roles = (r: string[]) => ["GET", "/api/auth/me", { tenant: "t", user: { id: "u", email: "a@b.c", full_name: "A", is_admin: false, roles: r }, packages: [] }] as [string, string, unknown];
const server = (me: string[], extra: Parameters<typeof fakeServer>[0] = []) =>
  fakeServer([
    ["POST", "/api/v1/SocialPost/_calc", calc],
    ...extra,
    roles(me),
    ["POST", "/api/v1/SocialAccount/x/actions/rules", rules],
    ["GET", "/api/v1/SocialPost/p1/tags", { tags: [] }],
    ["GET", "/api/v1/SocialPost/p1/approval", null],
    ["GET", "/api/v1/SocialPost/p1/links", { links: [] }],
    ["GET", "/api/v1/SocialPost/p1", { id: "p1", version: 1, headline: "Spring", brand: "b1", text: "Spring is here", status: "Draft", targets: [{ account: "x", kind: "", settings: {} }], items: [], media: [] }],
    ["GET", "/api/v1/SocialWritingPreset", { rows: [{ id: "pr1", preset_name: "Shorter", instruction: "Make it shorter" }] }],
    ["GET", "/api/v1/SocialBrand", { rows: [{ id: "b1", brand_name: "Northwind" }] }],
    ["GET", "/api/v1/SocialAccount", { rows: [{ id: "x", account_name: "North Short", brand: "b1", network: "Shortline", handle: "n", state: "Connected", active: true }] }],
    ["GET", "/api/v1/Social", { rows: [] }],
    ["GET", "/api/tags", { tags: [] }],
  ]);
const press = async (el: HTMLElement | undefined | null) => {
  expect(el).toBeTruthy();
  await act(async () => el!.click());
  await settle();
};

describe("writing help", () => {
  it("a rewrite is a proposal: the post is unchanged until it is taken", async () => {
    const fake = server(["Social Writing Help"], [["POST", "/api/v1/SocialPost/p1/actions/rewrite", { text: "Spring!" }]]);
    const host = await draw(<ComposerRoute />, "/compose/:id", "/compose/p1");
    await until(() => !!host.querySelector("#wh-preset"));
    const sel = host.querySelector<HTMLSelectElement>("#wh-preset")!;
    await until(() => sel.options.length > 1);
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(sel, "pr1");
      sel.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await press(byText(host, "button", "Rewrite it"));
    expect(fake.seen.find((s) => s.line.endsWith("/rewrite"))?.body).toEqual({ preset: "pr1" });
    expect(host.querySelector('[aria-label="What came back"]')?.textContent).toContain("Spring!");
    expect((host.querySelector("#original-item-0") as HTMLTextAreaElement).value).toBe("Spring is here");
    await press(byText(host, "button", "Replace the original's words"));
    expect((host.querySelector("#original-item-0") as HTMLTextAreaElement).value).toBe("Spring!");
  });

  it("adapt shows before and after for each account and fills its version only when taken", async () => {
    server(["Social Writing Help"], [["POST", "/api/v1/SocialPost/p1/actions/adapt", { versions: [{ account: "x", account_name: "North Short", text: "Spring, short", length: 13, max: 100, fits: true }] }]]);
    const host = await draw(<ComposerRoute />, "/compose/:id", "/compose/p1");
    await until(() => !!byText(host, "button", "Adapt for each account"));
    await press(byText(host, "button", "Adapt for each account"));
    const card = host.querySelector('[aria-label="What came back"]')!.textContent!;
    expect(card).toContain("Before: Spring is here");
    expect(card).toContain("After: Spring, short");
    expect(host.querySelectorAll('[role=tab]')).toHaveLength(1);
    await press(byText(host, "button", "Put each in its account's version"));
    expect(host.querySelectorAll('[role=tab]')).toHaveLength(2);
  });

  it("says the server's sentence when nothing is set, with the settings for a manager", async () => {
    server(["Social Writing Help"], [["POST", "/api/v1/SocialPost/p1/actions/hashtags", { error: "invalid", message: "no connection is set for writing help" }, 422]]);
    const host = await draw(<ComposerRoute />, "/compose/:id", "/compose/p1");
    await until(() => !!byText(host, "button", "Suggest hashtags"));
    await press(byText(host, "button", "Suggest hashtags"));
    expect(host.textContent).toContain("no connection is set for writing help");
  });

  it("is not there for a person without the role", async () => {
    server(["Social Author"]);
    const host = await draw(<ComposerRoute />, "/compose/:id", "/compose/p1");
    await until(() => !!host.querySelector("#original-item-0"));
    await settle(100);
    expect(host.textContent).not.toContain("Writing help");
  });
});

describe("a picture's place among the media lines", () => {
  it("counts the original's media, then each version's in the order of the accounts", () => {
    const d = emptyDraft();
    const m = (key: string) => ({ key, file: "/files/" + key, name: key, kind: "", alt: "", thumbnail: "" });
    d.original.items[0].media = [m("a"), m("b")];
    d.targets = [{ account: "x", kind: "", settings: {} }];
    d.versions.x = { items: [{ key: "i", text: "x", link: "", delay: 0, media: [m("c")] }] };
    expect([mediaIndex(d, "a"), mediaIndex(d, "b"), mediaIndex(d, "c"), mediaIndex(d, "zz")]).toEqual([0, 1, 2, -1]);
  });
});
