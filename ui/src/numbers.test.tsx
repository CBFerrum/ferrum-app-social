// @vitest-environment jsdom
// Numbers as drawn: the headline cards against the period before, followers by day with a line for each account, the report
// tables, the top posts by a number chosen, and a refusal in the server's words.

import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { NumbersRoute } from "./Numbers";
import { NativeRoute } from "./PostNumbers";
import { draw, fakeServer, settle, until } from "./testkit";

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const col = (name: string, label: string, type = "int") => ({ name, label, type });
const native = { id: "n1", account: "a1", native: true, text: "Posted from the phone", sent_at: "2030-06-03T10:00:00Z", impressions: 9000, engagements: 500, clicks: 1, permalink: "https://net.example/p/1" };
const reports = (extra: Parameters<typeof fakeServer>[0] = []) =>
  fakeServer([
    ...extra,
    ["GET", "/api/reports/social_period_compare", { columns: [col("measure", "Number", "data")], rows: [
      { measure: "publications", this_period: 12, period_before: 10, change: 2, change_pct: 20 },
      { measure: "impressions", this_period: 5400, period_before: 6000, change: -600, change_pct: -10 },
      { measure: "engagements", this_period: 300, period_before: 0, change: 300, change_pct: null },
      { measure: "clicks", this_period: 40, period_before: 40, change: 0, change_pct: 0 },
    ] }],
    ["GET", "/api/reports/social_by_account", { columns: [col("account", "Account", "data"), col("followers", "Followers")], rows: [{ account: "North Social", followers: 1200 }, { account: "North Pins", followers: 300 }] }],
    ["GET", "/api/reports/social_followers_by_day", { columns: [], rows: [{ day: "2030-06-01", account: "North Social", followers: 1100 }, { day: "2030-06-02", account: "North Social", followers: 1200 }] }],
    ["GET", "/api/reports/social_by_network", { columns: [col("network", "Network", "data"), col("publications", "Publications")], rows: [{ network: "Chirp", publications: 9 }] }],
    ["GET", "/api/reports/social_by_kind", { columns: [col("kind", "Kind", "data")], rows: [{ kind: "post" }] }],
    ["GET", "/api/reports/social_top_posts", { columns: [col("headline", "Post", "data"), col("engagements", "Engagements")], rows: [{ headline: "Spring launch", engagements: 120 }] }],
    ["GET", "/api/reports/social_best_times", { columns: [col("hour", "Hour", "data")], rows: [{ hour: "09:00" }] }],
    ["GET", "/api/v1/SocialPublication/n1", native],
    ["GET", "/api/v1/SocialPublication", { rows: [native] }],
    ["GET", "/api/v1/SocialAccount", { rows: [{ id: "a1", account_name: "North Social", brand: "b1" }] }],
    ["GET", "/api/v1/SocialInsight", { rows: [] }],
    ["GET", "/api/v1/SocialBrand", { rows: [{ id: "b1", brand_name: "Northwind" }] }],
  ]);

describe("numbers", () => {
  it("cards against the period before, followers with a line, and the tables", async () => {
    const fake = reports();
    const host = await draw(<NumbersRoute />, "/numbers", "/numbers");
    await until(() => host.textContent!.includes("Spring launch"));
    const t = host.textContent!;
    expect(t).toContain("5,400");
    expect(t).toContain("-10%");
    expect(t).toContain("+20%");
    expect(t).toContain("1,500"); // followers of the accounts added up
    expect(t).toContain("Chirp");
    expect(host.querySelectorAll("svg.spark")).toHaveLength(1);
    const asked = fake.seen.find((s) => s.line.startsWith("GET /api/reports/social_period_compare"))!;
    const q = new URL(asked.line.split(" ")[1], "http://x").searchParams;
    expect(Date.parse(q.get("to_date")!) - Date.parse(q.get("from_date")!)).toBe(27 * 86_400_000);
  });

  it("asks the top posts by the number chosen", async () => {
    const fake = reports();
    const host = await draw(<NumbersRoute />, "/numbers", "/numbers");
    await until(() => host.textContent!.includes("Spring launch"));
    const sel = host.querySelector<HTMLSelectElement>('select[aria-label="Top posts by"]')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(sel, "clicks");
      sel.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await settle();
    expect(fake.seen.some((s) => s.line.includes("social_top_posts") && s.line.includes("by=clicks"))).toBe(true);
  });

  it("says a refused report in the server's words", async () => {
    reports([["GET", "/api/reports/social_by_network", { error: "forbidden", message: "You do not have permission to do that." }, 403]]);
    const host = await draw(<NumbersRoute />, "/numbers", "/numbers");
    await until(() => host.textContent!.includes("Spring launch"));
    expect(host.textContent).toMatch(/access to this/);
  });
});

describe("posts made on the network", () => {
  it("are among the top posts with a mark and a link to their numbers, and in a table of their own by account", async () => {
    const fake = reports();
    const host = await draw(<NumbersRoute />, "/numbers", "/numbers");
    await until(() => host.textContent!.includes("Posted from the phone"));
    const link = host.querySelector<HTMLAnchorElement>('a[href="/native/n1"]');
    expect(link).toBeTruthy();
    expect(link!.parentElement!.textContent).toContain("Made on the network");
    // the native one (500) outranks the report's (120)
    const rowsText = [...host.querySelectorAll('section[aria-label="Top posts"] tbody tr')].map((r) => r.textContent);
    expect(rowsText[0]).toContain("Posted from the phone");
    expect(host.querySelector('section[aria-label="Made on the network"]')?.textContent).toContain("North Social");
    expect(fake.seen.some((s) => s.line.includes("SocialPublication") && decodeURIComponent(s.line).includes('"native","=",true'))).toBe(true);
  });

  it("opening one shows its numbers and where it is, not the composer", async () => {
    reports();
    const host = await draw(<NativeRoute />, "/native/:id", "/native/n1");
    await until(() => host.textContent!.includes("Posted from the phone"));
    expect(host.textContent).toContain("Made on the network");
    expect(host.textContent).toContain("9,000");
    expect(host.querySelector("#post-headline")).toBeNull();
    expect(host.querySelector('a[href="https://net.example/p/1"]')).toBeTruthy();
  });
});
