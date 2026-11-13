// @vitest-environment jsdom
// Accounts as they are drawn: by brand with the state and its note, a check and its refusal in the server's words,
// the sign-in that leaves the page, the picker after a sign-in that reaches several pages, and the order.

import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { AccountsRoute, leaveFor, reorder } from "./Accounts";
import { byText, draw, fakeServer, settle, until } from "./testkit";

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const brands = { rows: [{ id: "b1", brand_name: "Northwind" }, { id: "b2", brand_name: "Contoso" }] };
const account = (id: string, name: string, brand: string, state: string, extra: Record<string, unknown> = {}) => ({ id, account_name: name, brand, network: "Mastodon", handle: name.toLowerCase(), state, state_note: null, active: true, sort_order: 0, ...extra });
const list = {
  rows: [
    account("a1", "North Social", "b1", "Connected", { network_id: "p-1" }),
    account("a2", "North Pins", "b1", "Needs Reconnecting", { state_note: "the provider no longer accepts the sign-in: connect the account again", sort_order: 1 }),
    account("a3", "Conto", "b2", "Not Connected"),
  ],
};
const click = async (el: HTMLElement | undefined) => {
  expect(el).toBeTruthy();
  await act(async () => el!.click());
  await settle();
};

describe("accounts", () => {
  it("lists them by brand with their state and the server's note", async () => {
    fakeServer([["GET", "/api/v1/SocialBrand", brands], ["GET", "/api/v1/SocialAccount", list]]);
    const host = await draw(<AccountsRoute />, "/accounts", "/accounts");
    const groups = [...host.querySelectorAll("section h2")].map((h) => h.textContent);
    expect(groups).toEqual(["Contoso", "Northwind"]); // by the order of their accounts
    expect(host.textContent).toContain("Needs reconnecting");
    expect(host.textContent).toContain("Not connected");
    expect(host.textContent).toContain("the provider no longer accepts the sign-in: connect the account again");
  });

  it("checks an account, and shows the server's refusal as it is", async () => {
    const fake = fakeServer([
      ["POST", "/api/v1/SocialAccount/a2/actions/check", { error: "invalid", message: "Mastodon does not offer account" }, 422],
      ["POST", "/api/v1/SocialAccount/a1/actions/check", { state: "Connected", note: null, read: { display_name: "North" } }],
      ["GET", "/api/v1/SocialBrand", brands],
      ["GET", "/api/v1/SocialAccount", list],
    ]);
    const host = await draw(<AccountsRoute />, "/accounts", "/accounts");
    const cards = [...host.querySelectorAll("li.card")];
    const checks = cards.map((c) => byText(c as HTMLElement, "button", "Check"));
    await click(checks[1]);
    expect(fake.seen.some((s) => s.line === "POST /api/v1/SocialAccount/a1/actions/check")).toBe(true);
    await click(checks[2]);
    expect(cards[2].textContent).toContain("Mastodon does not offer account");
  });

  it("a sign-in that leaves the page: the connect answer's path is posted, and the address it gives is followed", async () => {
    const assign = vi.fn();
    vi.stubGlobal("location", { ...window.location, assign });
    const fake = fakeServer([
      ["POST", "/api/connections/c1/connect", { url: "https://provider.example/authorize?state=1" }],
      ["POST", "/api/v1/SocialAccount/a3/actions/connect", { connect: { method: "POST", path: "/api/connections/c1/connect" } }],
      ["GET", "/api/v1/SocialBrand", brands],
      ["GET", "/api/v1/SocialAccount", list],
    ]);
    const host = await draw(<AccountsRoute />, "/accounts", "/accounts");
    await click(byText(host.querySelectorAll("li.card")[0] as HTMLElement, "button", "Connect"));
    expect(fake.seen.some((s) => s.line === "POST /api/connections/c1/connect")).toBe(true);
    expect(assign).toHaveBeenCalledWith("https://provider.example/authorize?state=1");
  });

  it("after a sign-in that reaches several pages: choose one or more, each becomes an account", async () => {
    const fake = fakeServer([
      ["POST", "/api/v1/SocialAccount/a1/actions/reachable", { accounts: [
        { id: "p-1", name: "North Page", handle: "north", picture: null, kind: "page", account: "a1" },
        { id: "p-2", name: "South Page", handle: "south", picture: null, kind: "page", account: null },
      ] }],
      ["POST", "/api/v1/SocialAccount/a1/actions/choose", { accounts: [{ id: "a1", account_name: "North Social", network_id: "p-1", created: false }, { id: "a9", account_name: "South Page", network_id: "p-2", created: true }] }],
      ["GET", "/api/v1/SocialBrand", brands],
      ["GET", "/api/v1/SocialAccount", list],
    ]);
    const host = await draw(<AccountsRoute />, "/accounts", "/accounts");
    await click(byText(host.querySelectorAll("li.card")[1] as HTMLElement, "button", "Choose accounts"));
    await until(() => document.body.textContent!.includes("South Page"));
    const boxes = [...document.querySelectorAll<HTMLInputElement>("dialog input[type=checkbox]")];
    expect(boxes.map((b) => b.checked)).toEqual([true, false]);
    await act(async () => boxes[1].click());
    await click(byText(document.body, "button", "Use the chosen"));
    const sent = fake.seen.find((s) => s.line.endsWith("/actions/choose"));
    expect(sent?.body).toEqual({ picked: ["p-1", "p-2"] });
  });
});

describe("the pure parts", () => {
  it("the order after moving one place", () => {
    const rows = [{ id: "a", sort_order: 0 }, { id: "b", sort_order: 0 }, { id: "c", sort_order: 0 }];
    expect(reorder(rows, "c", -1)).toEqual([["c", 1], ["b", 2]]);
    expect(reorder(rows, "a", -1)).toEqual([]);
  });
  it("where a sign-in goes", () => {
    expect(leaveFor({ url: "https://x.example/a" })).toBe("https://x.example/a");
    expect(leaveFor({ authorize_url: "https://y.example" })).toBe("https://y.example");
    expect(leaveFor({})).toBeNull();
    expect(leaveFor(null)).toBeNull();
  });
});
