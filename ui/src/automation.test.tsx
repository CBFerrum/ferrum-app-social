// @vitest-environment jsdom
// Feeds and plugs with what they did, and the person's notice preferences (an opt-out row per notice turned off).

import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { AutomationRoute } from "./Automation";
import { NoticesRoute } from "./Notices";
import { byText, draw, fakeServer, settle, until } from "./testkit";

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const me = ["GET", "/api/auth/me", { tenant: "t", user: { id: "u", email: "Me@Example.org", full_name: "Me", is_admin: false, roles: [] }, packages: [] }] as [string, string, unknown];
const press = async (el: HTMLElement | undefined | null) => {
  expect(el).toBeTruthy();
  await act(async () => el!.click());
  await settle();
};

describe("feeds and plugs", () => {
  it("lists them and, opened, what each did: a post made, or why not", async () => {
    fakeServer([
      ["GET", "/api/v1/SocialFeedItem", { rows: [{ id: "i1", feed: "f1", title: "Spring is here", state: "Made", post: "p1", published_at: "2030-06-01T08:00:00Z" }, { id: "i2", feed: "f1", title: "Old news", state: "Skipped", note: "older than the feed", published_at: "2020-01-01T00:00:00Z" }] }],
      ["GET", "/api/v1/SocialFeed", { rows: [{ id: "f1", feed_name: "Blog", active: true, last_error: "the feed answered 503" }] }],
      ["GET", "/api/v1/SocialPlugRun", { rows: [] }],
      ["GET", "/api/v1/SocialPlug", { rows: [{ id: "g1", plug_name: "Repost winners", active: false }] }],
    ]);
    const host = await draw(<AutomationRoute />, "/automation", "/automation");
    await until(() => host.textContent!.includes("Blog"));
    expect(host.textContent).toContain("the feed answered 503");
    await press(byText(host, "button", "Runs"));
    await until(() => host.textContent!.includes("Spring is here"));
    expect(host.textContent).toContain("older than the feed");
    expect(host.querySelector('a[href="/compose/p1"]')).toBeTruthy();
    expect(host.textContent).toContain("Repost winners");
  });

  it("a refused read says so", async () => {
    fakeServer([["GET", "/api/v1/", { error: "forbidden", message: "no" }, 403]]);
    const host = await draw(<AutomationRoute />, "/automation", "/automation");
    await until(() => /access/.test(host.textContent!));
  });
});

describe("notice preferences", () => {
  it("a notice is on until an opt-out exists; turning one off makes the row, on again removes it", async () => {
    const fake = fakeServer([
      me,
      ["POST", "/api/v1/SocialNoticeOptOut", { id: "o2" }],
      ["DELETE", "/api/v1/SocialNoticeOptOut/o1", {}, 204],
      ["GET", "/api/v1/SocialNoticeOptOut", { rows: [{ id: "o1", person: "me@example.org", notice: "digest" }] }],
    ]);
    const host = await draw(<NoticesRoute />, "/notices", "/notices");
    await until(() => host.querySelectorAll("input[type=checkbox]").length === 6);
    const boxes = [...host.querySelectorAll<HTMLInputElement>("input[type=checkbox]")];
    expect(boxes.map((b) => b.checked)).toEqual([true, true, true, true, true, false]);
    await press(boxes[1]);
    expect(fake.seen.find((s) => s.line === "POST /api/v1/SocialNoticeOptOut")?.body).toEqual({ person: "me@example.org", notice: "failed" });
    await press(boxes[5]);
    expect(fake.seen.some((s) => s.line === "DELETE /api/v1/SocialNoticeOptOut/o1")).toBe(true);
  });

  it("shows the server's words when a change is refused", async () => {
    fakeServer([me, ["POST", "/api/v1/SocialNoticeOptOut", { error: "invalid", message: "You may only set your own notices" }, 422], ["GET", "/api/v1/SocialNoticeOptOut", { rows: [] }]]);
    const host = await draw(<NoticesRoute />, "/notices", "/notices");
    await until(() => host.querySelectorAll("input[type=checkbox]").length === 6);
    await press(host.querySelector<HTMLInputElement>("input[type=checkbox]"));
    expect(host.textContent).toContain("You may only set your own notices");
  });
});
