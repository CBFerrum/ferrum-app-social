// @vitest-environment jsdom
// Posting times as they are drawn: a brand's weekly times (add, switch off), the next places and what is in them, a post
// moved to a free place (reschedule), a draft put in the queue (the time it took shown), and the server's refusal.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { resetBootstrap, setTimeZone } from "@ferrum/client";
import { isTime, QueueRoute, timesFor } from "./Queue";
import { byText, draw, fakeServer, settle, until } from "./testkit";

beforeEach(() => setTimeZone("UTC"));
afterEach(() => {
  resetBootstrap();
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const soon = new Date(Date.now() + 2 * 86_400_000);
const at = (h: number) => `${soon.toISOString().slice(0, 10)}T${String(h).padStart(2, "0")}:00:00.000Z`;
const times = { rows: [{ id: "t1", version: 2, brand: "b1", weekday: "Every Day", time: "10:00", active: true }, { id: "t2", version: 1, brand: "b1", weekday: "Friday", time: "17:30", active: false }] };
const server = (extra: Parameters<typeof fakeServer>[0] = [], posts: unknown[] = [{ id: "p1", headline: "Spring sale", status: "Approved", scheduled_at: at(10), brand: "b1" }]) =>
  fakeServer([
    ...extra,
    ["GET", "/api/v1/SocialPostingTime", times],
    ["GET", "/api/v1/SocialPost", (url: URL) => ({ rows: url.search.includes("Draft") ? [{ id: "d1", headline: "A draft", status: "Draft" }] : posts })],
    ["GET", "/api/v1/SocialBrand", { rows: [{ id: "b1", brand_name: "Northwind" }] }],
    ["GET", "/api/v1/SocialAccount", { rows: [{ id: "x", account_name: "North Short", brand: "b1", state: "Connected", active: true }] }],
  ]);
const press = async (el: HTMLElement | null | undefined) => {
  expect(el).toBeTruthy();
  await act(async () => el!.click());
  await settle();
};

describe("posting times", () => {
  it("lists a brand's weekly times and the places they make with what is in them", async () => {
    server();
    const host = await draw(<QueueRoute />, "/queue", "/queue");
    await until(() => host.querySelectorAll("[data-at]").length > 0);
    expect(host.textContent).toContain("Every day");
    expect(host.textContent).toContain("17:30");
    const there = host.querySelector(`[data-at="${at(10)}"]`)!;
    expect(there.textContent).toContain("Spring sale");
    expect(host.querySelector(`[data-at="${at(10).replace("T10", "T11")}"]`)).toBeNull();
    expect(host.querySelectorAll("[data-at]").length).toBeGreaterThanOrEqual(10);
  });

  it("adds a time with the brand it belongs to, switches one off by its version", async () => {
    const fake = server([
      ["POST", "/api/v1/SocialPostingTime", { id: "t3" }],
      ["PATCH", "/api/v1/SocialPostingTime/t1", { id: "t1" }],
    ]);
    const host = await draw(<QueueRoute />, "/queue", "/queue");
    await press(byText(host, "button", "Add the time"));
    expect(fake.seen.find((s) => s.line === "POST /api/v1/SocialPostingTime")?.body).toEqual({ brand: "b1", account: null, weekday: "Every Day", time: "09:00", active: true });
    await press(host.querySelector<HTMLElement>('input[aria-label="Every day 10:00 is on"]'));
    expect(fake.seen.find((s) => s.line === "PATCH /api/v1/SocialPostingTime/t1")?.body).toEqual({ active: false });
  });

  it("puts a draft in the queue; a refusal is shown as the server says it", async () => {
    const fake = server([["POST", "/api/v1/SocialPost/d1/actions/add_to_queue", { scheduled_at: at(10), status: "Draft" }]]);
    let host = await draw(<QueueRoute />, "/queue", "/queue");
    await until(() => !!byText(host, "button", "Add to queue"));
    await press(byText(host, "button", "Add to queue"));
    // (the answer's time is said in a toast: "In the queue for …")
    expect(fake.seen.some((s) => s.line === "POST /api/v1/SocialPost/d1/actions/add_to_queue")).toBe(true);
    document.body.innerHTML = "";
    fakeServer([
      ["POST", "/api/v1/SocialPost/d1/actions/add_to_queue", { error: "invalid", message: "no posting times are set for its accounts or its brand: add some under Posting Times" }, 422],
      ["GET", "/api/v1/SocialPostingTime", { rows: [] }],
      ["GET", "/api/v1/SocialPost", (url: URL) => ({ rows: url.search.includes("Draft") ? [{ id: "d1", headline: "A draft", status: "Draft" }] : [] })],
      ["GET", "/api/v1/SocialBrand", { rows: [{ id: "b1", brand_name: "Northwind" }] }],
      ["GET", "/api/v1/SocialAccount", { rows: [] }],
    ]);
    host = await draw(<QueueRoute />, "/queue", "/queue");
    await until(() => !!byText(host, "button", "Add to queue"));
    await press(byText(host, "button", "Add to queue"));
    expect(host.querySelector(".alert-error")?.textContent).toContain("no posting times are set for its accounts or its brand");
  });

  it("moves a post to a free place by the reschedule action", async () => {
    const fake = server([["POST", "/api/v1/SocialPost/p1/actions/reschedule", { scheduled_at: at(10), status: "Approved" }]]);
    const host = await draw(<QueueRoute />, "/queue", "/queue");
    await until(() => !!host.querySelector("select[aria-label^='Move Spring sale']"));
    const select = host.querySelector<HTMLSelectElement>("select[aria-label^='Move Spring sale']")!;
    const target = select.options[1].value;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(select, target);
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await settle();
    expect(fake.seen.find((s) => s.line.endsWith("/p1/actions/reschedule"))?.body).toEqual({ at: target });
  });
});

describe("the pure parts", () => {
  it("an account with times of its own uses only those; without, the brand's", () => {
    const all = [{ id: "1", brand: "b" }, { id: "2", brand: "b", account: "x" }, { id: "3", brand: "c" }] as never[];
    expect(timesFor(all, "b", "").own.map((r: { id: string }) => r.id)).toEqual(["1"]);
    expect(timesFor(all, "b", "x").own.map((r: { id: string }) => r.id)).toEqual(["2"]);
    expect(timesFor(all, "b", "x").shared).toEqual([]);
    expect(timesFor(all, "b", "y").shared.map((r: { id: string }) => r.id)).toEqual(["1"]);
  });
  it("a time is HH:MM", () => {
    expect([isTime("09:30"), isTime("9:30"), isTime("24:00"), isTime("9.30")]).toEqual([true, true, false, false]);
  });
});
