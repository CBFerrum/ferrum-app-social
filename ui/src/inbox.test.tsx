// @vitest-environment jsdom
// The inbox desk: conversations with their filters sent to the server, the thread with the post it answers and our replies, a
// reply sent (and refused in the server's words), and the keys.

import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { filtersFor, InboxRoute } from "./Inbox";
import { byText, draw, fakeServer, settle, until } from "./testkit";

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const msgs = { rows: [
  { id: "m1", version: 1, author: "ann", kind: "Comment", status: "New", text: "Love this!", account: "a1", thread: "th-1", post_external_id: "ext-9", external_id: "c1", contact_entity: "Contact", contact_id: "c-7", created_at: "2030-06-01T10:00:00Z" },
  { id: "m2", version: 1, author: "bo", kind: "Direct", status: "Assigned", text: "Question about prices", account: "a1", created_at: "2030-06-01T11:00:00Z" },
] };
const server = (extra: Parameters<typeof fakeServer>[0] = []) =>
  fakeServer([
    ...extra,
    ["GET", "/api/auth/me", { tenant: "t", user: { id: "u", email: "me@example.org", full_name: "Me", is_admin: false, roles: [] }, packages: [] }],
    ["GET", "/api/v1/Contact/c-7", { id: "c-7", full_name: "Ann Archer", email: "ann@example.com" }],
    ["GET", "/api/v1/SocialReply", { rows: [{ id: "r1", message: "m1", author: "me@example.org", text: "Thank you!", created_at: "2030-06-01T10:30:00Z" }] }],
    ["GET", "/api/v1/SocialPublication", { rows: [{ id: "pb1", post: "p1", external_id: "ext-9" }] }],
    ["GET", "/api/v1/SocialPost/p1", { id: "p1", headline: "Spring sale" }],
    ["GET", "/api/v1/SocialSavedReply", { rows: [{ id: "sr1", title: "Thanks", text: "Thanks for writing to us." }] }],
    ["GET", "/api/v1/SocialMessage", msgs],
    ["GET", "/api/v1/SocialBrand", { rows: [{ id: "b1", brand_name: "Northwind" }] }],
    ["GET", "/api/v1/SocialAccount", { rows: [{ id: "a1", account_name: "North Social", brand: "b1", state: "Connected", active: true }] }],
  ]);
const press = async (el: HTMLElement | undefined | null) => {
  expect(el).toBeTruthy();
  await act(async () => el!.click());
  await settle();
};
const type = async (el: HTMLElement | null, value: string) => {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(el, value);
    el!.dispatchEvent(new Event("input", { bubbles: true }));
  });
};
const key = async (k: string) => {
  await act(async () => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true }));
  });
  await settle();
};

describe("the inbox", () => {
  it("shows the conversation, the post it answers, our reply and the contact", async () => {
    server();
    const host = await draw(<InboxRoute />, "/inbox", "/inbox");
    await until(() => host.textContent!.includes("Ann Archer"));
    expect(host.textContent).toContain("Love this!");
    expect(host.textContent).toContain("Thank you!");
    expect(host.textContent).toContain("Spring sale");
    expect(host.querySelector('a[href="/compose/p1"]')).toBeTruthy();
  });

  it("sends a reply by the reply action; a saved reply fills the box", async () => {
    const fake = server([["POST", "/api/v1/SocialMessage/m1/actions/reply", { reply: "r2" }]]);
    const host = await draw(<InboxRoute />, "/inbox", "/inbox");
    await until(() => !!host.querySelector("#inbox-reply"));
    const sel = host.querySelector<HTMLSelectElement>('select[aria-label="A saved reply"]')!;
    await until(() => sel.options.length > 1);
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(sel, "sr1");
      sel.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect((host.querySelector("#inbox-reply") as HTMLTextAreaElement).value).toBe("Thanks for writing to us.");
    await press(byText(host, "button", "Send the reply"));
    expect(fake.seen.find((s) => s.line.endsWith("/m1/actions/reply"))?.body).toEqual({ reply: "Thanks for writing to us." });
  });

  it("shows the server's refusal and keeps the words typed", async () => {
    server([["POST", "/api/v1/SocialMessage/m1/actions/reply", { error: "invalid", message: "North Social's connection cannot reply" }, 422]]);
    const host = await draw(<InboxRoute />, "/inbox", "/inbox");
    await until(() => !!host.querySelector("#inbox-reply"));
    await type(host.querySelector("#inbox-reply"), "Hello");
    await press(byText(host, "button", "Send the reply"));
    expect(host.textContent).toContain("North Social's connection cannot reply");
    expect((host.querySelector("#inbox-reply") as HTMLTextAreaElement).value).toBe("Hello");
  });

  it("j moves to the next conversation, e closes the open one", async () => {
    const fake = server([["PATCH", "/api/v1/SocialMessage/m2", { id: "m2" }]]);
    const host = await draw(<InboxRoute />, "/inbox", "/inbox");
    await until(() => !!host.querySelector("#inbox-reply"));
    await key("j");
    await until(() => host.textContent!.includes("Question about prices") && !!host.querySelector('[aria-label="The conversation"]')?.textContent?.includes("Question about prices"));
    await key("e");
    expect(fake.seen.find((s) => s.line === "PATCH /api/v1/SocialMessage/m2")?.body).toEqual({ status: "Closed" });
  });
});

describe("the filters", () => {
  it("open ones by default; account, brand through the account, kind, mine", () => {
    expect(filtersFor({ account: "", brand: "", kind: "", status: "", mine: false }, "me@x")).toEqual([["status", "in", ["New", "Assigned", "Answered"]]]);
    expect(filtersFor({ account: "a1", brand: "b1", kind: "Direct", status: "Closed", mine: true }, "me@x")).toEqual([["account", "=", "a1"], ["account.brand", "=", "b1"], ["kind", "=", "Direct"], ["status", "=", "Closed"], ["assigned_to", "=", "me@x"]]);
  });
});
