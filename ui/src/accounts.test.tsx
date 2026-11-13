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

  it("a sign-in that leaves the page: the connect answer's path is posted and the address it gives opens in a new tab; coming back checks the account and offers the picker", async () => {
    const open = vi.fn(() => ({}) as Window);
    vi.stubGlobal("open", open);
    const fake = fakeServer([
      ["POST", "/api/connections/c1/connect", { url: "https://provider.example/authorize?state=1" }],
      ["POST", "/api/v1/SocialAccount/a3/actions/connect", { connect: { method: "POST", path: "/api/connections/c1/connect" } }],
      ["GET", "/api/v1/SocialBrand", brands],
      ["GET", "/api/v1/SocialAccount", list],
    ]);
    const host = await draw(<AccountsRoute />, "/accounts", "/accounts");
    await click(byText(host.querySelectorAll("li.card")[0] as HTMLElement, "button", "Connect"));
    expect(fake.seen.some((s) => s.line === "POST /api/connections/c1/connect")).toBe(true);
    expect(open).toHaveBeenCalledWith("https://provider.example/authorize?state=1", "_blank");
    // the person comes back: the account is checked (Connected now) and, the sign-in reaching two pages, the picker opens
    fake.routes.unshift(
      ["POST", "/api/v1/SocialAccount/a3/actions/check", { state: "Connected", note: null, read: {} }],
      ["POST", "/api/v1/SocialAccount/a3/actions/reachable", { accounts: [{ id: "p-1", name: "A", handle: null, picture: null, kind: null, account: null }, { id: "p-2", name: "B", handle: null, picture: null, kind: null, account: null }] }],
    );
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
    });
    await until(() => fake.seen.some((s) => s.line.endsWith("/a3/actions/reachable")));
    await until(() => document.body.textContent!.includes("Use the chosen"));
    expect(fake.seen.some((s) => s.line.endsWith("/a3/actions/check"))).toBe(true);
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
    expect(leaveFor({})).toBeNull();
    expect(leaveFor(null)).toBeNull();
  });
});

describe("adding an account from the networks offered", () => {
  const conn = (name: string, enabled: boolean, ops: object, extra: object = {}) => ({ id: enabled ? name : null, name, label: name, enabled, offered: !enabled, secrets: [], description: { apps: ["social"], operations: ops, auth: { type: "oauth" } }, ...extra });
  const listed = { connections: [conn("mastodon", true, { publish: {} }), conn("youtube", false, { publish: {} }), conn("stockpics", true, { search_media: {} }), conn("wordpress", false, { publish: {} }, { secrets: ["site_password"] })] };

  it("an administrator sees the networks (not the services), what each needs, and is led to the platform's screen for one that is off", async () => {
    const fake = fakeServer([["GET", "/api/admin/connections", listed], ["GET", "/api/v1/SocialProfile", { rows: [] }], ["GET", "/api/v1/SocialBrand", brands], ["GET", "/api/v1/SocialAccount", list]]);
    const host = await draw(<AccountsRoute />, "/accounts", "/accounts");
    await click(byText(host, "button", "Add an account"));
    await until(() => document.body.textContent!.includes("youtube"));
    const dlg = document.body.textContent!;
    expect(dlg).toContain("mastodon");
    expect(dlg).not.toContain("stockpics");
    expect(dlg).toContain("Secrets: site_password");
    expect(dlg).toContain("A sign-in with the network, once it is turned on");
    const link = [...document.querySelectorAll<HTMLAnchorElement>("dialog a")].find((a) => a.getAttribute("href") === "/admin?tab=connections");
    expect(link).toBeTruthy();
    await click(byText(document.body, "button", "Use it"));
    expect((document.querySelector("#acc-conn") as HTMLInputElement).value).toBe("mastodon");
    void fake;
  });

  it("someone who is not an administrator is told plainly who can, and may still give a connection's name", async () => {
    fakeServer([["GET", "/api/admin/connections", { error: "forbidden", message: "no" }, 403], ["GET", "/api/v1/SocialProfile", { rows: [] }], ["GET", "/api/v1/SocialBrand", brands], ["GET", "/api/v1/SocialAccount", list]]);
    const host = await draw(<AccountsRoute />, "/accounts", "/accounts");
    await click(byText(host, "button", "Add an account"));
    await until(() => document.body.textContent!.includes("Only an administrator"));
    await click(byText(document.body, "button", "I know the connection's name"));
    expect(document.querySelector("#acc-conn")).toBeTruthy();
  });
});
