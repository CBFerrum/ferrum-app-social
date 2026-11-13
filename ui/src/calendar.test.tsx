// @vitest-environment jsdom
// The calendar as it is drawn: a month of chips, a post dragged to another day (the reschedule action; the server's refusal puts
// the chip back), a free posting time in the week as an empty place, the quick look with its way to move a post without dragging,
// and the keyboard between days.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { resetBootstrap, setTimeZone } from "@ferrum/client";
import { CalendarRoute, filtersFor, repeatWords } from "./Calendar";
import { byText, draw, fakeServer, settle, until } from "./testkit";

beforeEach(() => setTimeZone("UTC"));
afterEach(() => {
  resetBootstrap();
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const brands = { rows: [{ id: "b1", brand_name: "Northwind" }] };
const accounts = { rows: [{ id: "x", account_name: "North Short", brand: "b1", network: "Shortline", handle: "n", state: "Connected", active: true, sort_order: 0 }] };
const post = (id: string, headline: string, at: string, status = "Approved", extra: Record<string, unknown> = {}) => ({ id, headline, scheduled_at: at, status, brand: "b1", targets: [{ account: "x" }], ...extra });
const june = { rows: [post("p1", "Spring sale", "2030-06-12T09:00:00Z"), post("p2", "Weekly tip", "2030-06-14T15:30:00Z", "Draft", { repeat_every: 1, repeat_unit: "Weeks", repeat_count: 4 })] };
const server = (extra: Parameters<typeof fakeServer>[0] = []) =>
  fakeServer([
    ...extra,
    ["GET", "/api/v1/SocialPostingTime", { rows: [{ brand: "b1", weekday: "Every Day", time: "10:00", active: true }] }],
    ["GET", "/api/v1/SocialPost", june],
    ["GET", "/api/v1/SocialBrand", brands],
    ["GET", "/api/v1/SocialAccount", accounts],
    ["GET", "/api/tags", { tags: ["spring"] }],
  ]);
const at = "/calendar?d=2030-06-12";
const chip = (host: HTMLElement, id: string) => host.querySelector<HTMLElement>(`[data-post="${id}"]`)!;
const fire = async (el: Element, type: string) => {
  await act(async () => {
    el.dispatchEvent(new Event(type, { bubbles: true, cancelable: true }));
  });
};

describe("the calendar", () => {
  it("a month: posts as chips on their days with time and headline, a repeat marked, the server asked for the range", async () => {
    const fake = server();
    const host = await draw(<CalendarRoute />, "/calendar", at);
    expect(chip(host, "p1").closest("[data-day]")?.getAttribute("data-day")).toBe("2030-06-12");
    expect(chip(host, "p1").textContent).toContain("Spring sale");
    expect(chip(host, "p1").textContent).toContain("09:00");
    // the chip keeps its parts in order and never wider than its cell: time, headline (ellipsised by CSS), marks
    expect([...chip(host, "p1").children].map((c) => c.className)).toEqual(["cal-chip-time", "cal-chip-title", "cal-faces"]);
    expect(chip(host, "p2").textContent).toContain("↻");
    const asked = fake.seen.find((s) => s.line.startsWith("GET /api/v1/SocialPost?"))!;
    const filters = JSON.parse(new URL(asked.line.split(" ")[1], "http://x").searchParams.get("filters")!);
    // the weeks that touch June 2030, from the weekday this language's weeks begin on (Sunday here)
    expect(filters[0]).toEqual(["scheduled_at", "between", ["2030-05-26T00:00:00.000Z", "2030-07-07T00:00:00.000Z"]]);
  });

  it("dragging a post to another day is the reschedule action at the same time of day", async () => {
    const fake = server([["POST", "/api/v1/SocialPost/p1/actions/reschedule", { scheduled_at: "2030-06-18T09:00:00Z", status: "Approved" }]]);
    const host = await draw(<CalendarRoute />, "/calendar", at);
    await fire(chip(host, "p1"), "dragstart");
    await fire(host.querySelector('[data-day="2030-06-18"]')!, "drop");
    await settle();
    const sent = fake.seen.find((s) => s.line.endsWith("/p1/actions/reschedule"));
    expect(sent?.body).toEqual({ at: "2030-06-18T09:00:00.000Z" });
    expect(fake.seen.some((s) => s.line.startsWith("PATCH"))).toBe(false);
  });

  it("a refused move shows the server's words and the chip stays on its day", async () => {
    server([["POST", "/api/v1/SocialPost/p1/actions/reschedule", { error: "invalid", message: "a post is not moved to the past" }, 422]]);
    const host = await draw(<CalendarRoute />, "/calendar", at);
    await fire(chip(host, "p1"), "dragstart");
    await fire(host.querySelector('[data-day="2030-06-03"]')!, "drop");
    await settle();
    expect(host.textContent).toContain("a post is not moved to the past");
    expect(chip(host, "p1").closest("[data-day]")?.getAttribute("data-day")).toBe("2030-06-12");
  });

  it("a post that is out cannot be dragged", async () => {
    server([["GET", "/api/v1/SocialPost", { rows: [post("p3", "Out already", "2030-06-12T09:00:00Z", "Published")] }]]);
    const host = await draw(<CalendarRoute />, "/calendar", at);
    expect(chip(host, "p3").getAttribute("draggable")).toBe("false");
  });

  it("the week shows a free posting time as an empty place that composes a post at that time", async () => {
    server();
    const host = await draw(<CalendarRoute />, "/calendar", "/calendar?v=week&d=2030-06-12");
    const places = [...host.querySelectorAll<HTMLElement>(".cal-place")];
    expect(places).toHaveLength(7);
    expect(places[0].textContent).toContain("Free");
    expect(host.querySelectorAll(".cal-col")).toHaveLength(7);
  });

  it("the quick look: the accounts' copies, the repeat rule, and moving it without dragging", async () => {
    const fake = server([
      ["POST", "/api/v1/SocialPost/p2/actions/copies", { copies: [{ account: "x", account_name: "North Short", items: [{ text: "Tip of the week", length: 15, max: 100 }], problems: [] }] }],
      ["POST", "/api/v1/SocialPost/p2/actions/reschedule", { scheduled_at: "2030-06-15T08:00:00Z", status: "Draft" }],
      ["GET", "/api/v1/SocialPost/p2", post("p2", "Weekly tip", "2030-06-14T15:30:00Z", "Draft", { version: 1, repeat_every: 1, repeat_unit: "Weeks", repeat_count: 4 })],
    ]);
    const host = await draw(<CalendarRoute />, "/calendar", at);
    await act(async () => chip(host, "p2").click());
    await until(() => document.body.textContent!.includes("Tip of the week"));
    expect(document.body.textContent).toContain("Repeats every 1 weeks, 4 times in all");
    const input = document.querySelector<HTMLInputElement>("#ql-move")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "2030-06-15T08:00");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => byText(document.body, "button", "Move")!.click());
    await settle();
    expect(fake.seen.find((s) => s.line.endsWith("/p2/actions/reschedule"))?.body).toEqual({ at: "2030-06-15T08:00:00.000Z" });
  });

  it("arrow keys move between days", async () => {
    server();
    const host = await draw(<CalendarRoute />, "/calendar", at);
    const date = (d: string) => host.querySelector<HTMLElement>(`[data-day="${d}"] .cal-date`)!;
    date("2030-06-12").focus();
    await act(async () => {
      date("2030-06-12").dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    });
    await settle();
    expect(document.activeElement).toBe(date("2030-06-13"));
  });

  it("a failed read says so, with the server's words", async () => {
    server([["GET", "/api/v1/SocialPost", { error: "forbidden", message: "You do not have permission to do that." }, 403]]);
    const host = await draw(<CalendarRoute />, "/calendar", at);
    expect(host.textContent).toMatch(/permission|allowed|access/i);
  });
});

describe("the pure parts", () => {
  it("filters the server is asked with", () => {
    expect(filtersFor("a", "b", { brand: "b1", account: "x", status: "Failed", tag: "spring" })).toEqual([
      ["scheduled_at", "between", ["a", "b"]],
      ["brand", "=", "b1"],
      ["targets.account", "in", ["x"]],
      ["status", "in", ["Failed"]],
      ["_tag", "=", "spring"],
    ]);
  });
  it("the repeat rule in words", () => {
    expect(repeatWords({ id: "p", repeat_every: 2, repeat_unit: "Months", repeat_until: "2031-01-31" })).toContain("Repeats every 2 months, until");
    expect(repeatWords({ id: "p" })).toBe("");
  });
});

describe("a chip in a crowded cell", () => {
  it("shows at most two account marks and the number of the rest; the accessible name keeps the whole", async () => {
    const many = { id: "p1", headline: "Spring menu launch with a very long headline", scheduled_at: "2030-06-12T09:30:00Z", status: "Draft", brand: "b1", targets: ["x", "y", "z", "w"].map((a) => ({ account: a })) };
    server([["GET", "/api/v1/SocialAccount", { rows: ["x", "y", "z", "w"].map((a) => ({ id: a, account_name: `Acc ${a}`, brand: "b1", state: "Connected", active: true })) }]]);
    document.body.innerHTML = "";
    fakeServer([["GET", "/api/v1/SocialPostingTime", { rows: [] }], ["GET", "/api/v1/SocialPost", { rows: [many] }], ["GET", "/api/v1/SocialBrand", brands], ["GET", "/api/v1/SocialAccount", { rows: ["x", "y", "z", "w"].map((a) => ({ id: a, account_name: `Acc ${a}`, brand: "b1", state: "Connected", active: true })) }], ["GET", "/api/tags", { tags: [] }]]);
    const host = await draw(<CalendarRoute />, "/calendar", at);
    const c = chip(host, "p1");
    expect(c.querySelectorAll(".cal-faces .face")).toHaveLength(2);
    expect(c.querySelector(".cal-more-faces")?.textContent).toBe("+2");
    expect(c.getAttribute("aria-label")).toContain("Spring menu launch with a very long headline");
  });
});

describe("reading the posts", () => {
  it("uses the first answer's targets: one list call, not one per account", async () => {
    const fake = server();
    await draw(<CalendarRoute />, "/calendar", at);
    expect(fake.seen.filter((s) => s.line.startsWith("GET /api/v1/SocialPost?")).length).toBe(1);
  });
  it("falls back to a read per account only when no row has targets", async () => {
    const bare = { rows: [{ id: "p1", headline: "A", scheduled_at: "2030-06-12T09:00:00Z", status: "Draft", brand: "b1" }] };
    const fake = server([["GET", "/api/v1/SocialPost", bare]]);
    await draw(<CalendarRoute />, "/calendar", at);
    expect(fake.seen.filter((s) => s.line.startsWith("GET /api/v1/SocialPost?")).length).toBe(2);
  });
});
