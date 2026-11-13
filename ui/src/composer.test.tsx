// @vitest-environment jsdom
// The composer as it is drawn: an account chosen, the settings its rules ask (by kind), the live count against its network's
// most, a save with the record's shape, the server's refusal as it comes, and a saved post with the server's own count.

import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { ComposerRoute } from "./Composer";
import { mentionToken } from "./composer/Items";
import { byText, draw, fakeServer, settle, until } from "./testkit";

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const media = { kinds: ["image"], required: false, mixes: [], other: {}, alt: true, alt_required: false, thumbnail: false, max: 4 };
// What the account's `rules` action answers (rules.rs `Rules`, serialised with its defaults).
const rules = {
  rules: {
    name: "Shortline",
    text: { count: "characters", max: 10, min: null, link_counts: null, format: null },
    media,
    kinds: [{ name: "post", label: "Post" }, { name: "pin", label: "Pin", follow_ups: { as: "none", media: false } }],
    follow_ups: { as: "thread", media: false },
    settings: [
      { key: "board", label: "Board", type: "select", options: [], choices: { operation: "boards" }, required: true, when: { kind: ["pin"] } },
      { key: "title", label: "Title", type: "text", options: [], required: false, when: {} },
    ],
    operations: {},
    first_comment: false,
    link_preview: false,
    scheduling: true,
  },
};
const brands = { rows: [{ id: "b1", brand_name: "Northwind" }] };
const accounts = { rows: [{ id: "x", account_name: "North Short", display_name: "North", brand: "b1", network: "Shortline", handle: "north", state: "Connected", active: true, sort_order: 0 }] };
const empty = { rows: [] };
// What `_calc` with want_checks answers (src/posts.rs `copies`): here a network that takes 10 characters and folds at 6.
const calc = (_url: URL, body: unknown) => {
  const b = body as { text: string };
  const n = [...b.text].length;
  return { text: b.text, checks: { copies: [{ account: "x", account_name: "North Short", network: "Shortline", kind: "post", follow_ups: "thread", items: [{ text: b.text, link: null, delay_minutes: 0, length: n, max: 10, fold: 6, media: [] }], first_comment: null, settings: {}, problems: n > 10 ? [`Shortline takes 10 characters, and this post is ${n}`] : [] }] } };
};
const server = (extra: Parameters<typeof fakeServer>[0] = []) =>
  fakeServer([
    ["POST", "/api/v1/SocialPost/_calc", calc],
    ...extra,
    ["POST", "/api/v1/SocialAccount/x/actions/rules", rules],
    ["GET", "/api/v1/SocialBrand", brands],
    ["GET", "/api/v1/SocialAccount", accounts],
    ["GET", "/api/v1/Social", empty],
    ["GET", "/api/tags", { tags: [] }],
  ]);

/** Types into a controlled field the way a person does. */
const type = async (el: HTMLElement | null, value: string) => {
  expect(el).toBeTruthy();
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : el instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
  await act(async () => {
    Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, value);
    el!.dispatchEvent(new Event(el instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
  });
};
const press = async (el: HTMLElement | undefined) => {
  expect(el).toBeTruthy();
  await act(async () => el!.click());
  await settle();
};
const field = (host: HTMLElement, id: string) => host.querySelector<HTMLElement>(`#${id}`);

describe("the composer", () => {
  it("a new post: choose an account, write, see the count, save with the record's shape", async () => {
    const fake = server([["POST", "/api/v1/SocialPost", { id: "p9", version: 1, status: "Draft" }]]);
    const host = await draw(<ComposerRoute />, "/compose", "/compose");
    await press(host.querySelector<HTMLInputElement>("input[type=checkbox]")!);
    await until(() => !!field(host, "original-item-0"));
    await type(field(host, "post-headline"), "Spring");
    await type(field(host, "original-item-0"), "Spring is here");
    // 14 characters against the network's 10, as its rules count them.
    await until(() => host.textContent!.includes("Shortline takes 10 characters, and this post is 14"));
    expect(host.textContent).toContain("14 of 10");
    // the preview folds at the rules' fold, "see more" opens it
    expect(host.querySelector(".preview-card")?.textContent).toContain("Spring… See more");
    await type(field(host, "original-item-0"), "Spring!");
    await until(() => host.textContent!.includes("Nothing in the way."));
    await press(byText(host, "button", "Save draft"));
    const sent = fake.seen.find((s) => s.line === "POST /api/v1/SocialPost");
    expect(sent?.body).toMatchObject({
      headline: "Spring",
      brand: "b1",
      text: "Spring!",
      scheduled_at: null,
      targets: [{ account: "x", kind: "", settings: {}, text: "" }],
      items: [],
      media: [],
    });
  });

  it("shows the server's words when a save is refused, and keeps what was written", async () => {
    server([["POST", "/api/v1/SocialPost", { error: "invalid", message: "Please check your input.", fields: [{ field: "text", message: "for North Short: Shortline takes 10 characters, and this post is 14" }] }, 422]]);
    const host = await draw(<ComposerRoute />, "/compose", "/compose");
    await press(host.querySelector<HTMLInputElement>("input[type=checkbox]")!);
    await until(() => !!field(host, "original-item-0"));
    await type(field(host, "post-headline"), "Spring");
    await type(field(host, "original-item-0"), "Spring is here");
    await press(byText(host, "button", "Save draft"));
    expect(host.querySelector(".alert-error")?.textContent).toContain("for North Short: Shortline takes 10 characters, and this post is 14");
    expect((field(host, "original-item-0") as HTMLTextAreaElement).value).toBe("Spring is here");
  });

  it("asks a headline and an account before it sends anything", async () => {
    const fake = server();
    const host = await draw(<ComposerRoute />, "/compose", "/compose");
    await press(byText(host, "button", "Save draft"));
    expect(host.textContent).toContain("Choose at least one account");
    expect(fake.seen.some((s) => s.line.startsWith("POST /api/v1/SocialPost"))).toBe(false);
  });

  it("draws the settings from the rules: a kind's questions only when the kind is chosen, choices from the server", async () => {
    const fake = server([["POST", "/api/v1/SocialAccount/x/actions/choices", { choices: [{ value: "b-1", label: "Spring board" }, { value: "b-2", label: "Summer board" }] }]]);
    const host = await draw(<ComposerRoute />, "/compose", "/compose");
    await press(host.querySelector<HTMLInputElement>("input[type=checkbox]")!);
    await until(() => !!field(host, "kind-x"));
    expect(field(host, "set-x-title")).toBeTruthy();
    expect(field(host, "set-x-board")).toBeFalsy();
    await type(field(host, "kind-x"), "pin");
    await until(() => !!field(host, "set-x-board"));
    expect(host.textContent).toContain("Board*");
    await until(() => (field(host, "set-x-board") as HTMLSelectElement).options.length > 2);
    const asked = fake.seen.find((s) => s.line.endsWith("/choices"));
    expect(asked?.body).toMatchObject({ setting: "board", query: "", settings: {} });
    await type(field(host, "set-x-board"), JSON.stringify("b-2"));
    expect(host.textContent).not.toContain("Board is needed");
  });

  it("a draft with problems is kept; leaving Draft is refused in the server's words", async () => {
    const fake = server([
      ["POST", "/api/v1/SocialPost/p9/actions/send_for_review", { error: "invalid", message: "for North Short: Shortline takes 10 characters, and this post is 14" }, 422],
      ["POST", "/api/v1/SocialPost", { id: "p9", version: 1, status: "Draft" }],
      ["GET", "/api/v1/SocialPost/p9/tags", { tags: [] }],
      ["GET", "/api/v1/SocialPost/p9/approval", null],
      ["GET", "/api/v1/SocialPost/p9/links", { links: [] }],
      ["GET", "/api/v1/SocialPost/p9", { id: "p9", version: 1, headline: "Spring", brand: "b1", text: "Spring is here", status: "Draft", targets: [{ account: "x", kind: "", settings: {} }], items: [], media: [] }],
    ]);
    const host = await draw(<ComposerRoute />, "/compose/:id?", "/compose");
    await press(host.querySelector<HTMLInputElement>("input[type=checkbox]")!);
    await until(() => !!field(host, "original-item-0"));
    await type(field(host, "post-headline"), "Spring");
    await type(field(host, "original-item-0"), "Spring is here");
    await until(() => host.textContent!.includes("this post is 14"));
    await press(byText(host, "button", "Save draft"));
    expect(fake.seen.some((s) => s.line === "POST /api/v1/SocialPost")).toBe(true);
    expect(host.querySelector(".alert-error")?.textContent ?? "").not.toContain("takes 10 characters, and this post is 14\n");
    await press(byText(host, "button", "Send for review"));
    expect(fake.seen.some((s) => s.line.endsWith("/p9/actions/send_for_review"))).toBe(true);
    expect(host.textContent).toContain("for North Short: Shortline takes 10 characters, and this post is 14");
  });

  it("a saved post: the server's count beside the account, the review steps", async () => {
    server([
      ["GET", "/api/v1/SocialPost/p1/approval", { id: "r1", status: "pending", steps: [{ label: "Social Approver", state: "open", need: 1, approvers: ["u1"] }], decisions: [] }],
      ["GET", "/api/approvals", { to_decide: [], awaiting_others: [] }],
      ["GET", "/api/v1/SocialPost/p1/tags", { tags: ["spring"] }],
      ["GET", "/api/v1/SocialPost/p1/links", { links: [] }],
      ["GET", "/api/v1/SocialPost/p1", { id: "p1", version: 3, headline: "Spring", brand: "b1", text: "Spring is here", status: "In Review", add_signature: false, targets: [{ account: "x", kind: "", settings: {} }], items: [], media: [] }],
    ]);
    const host = await draw(<ComposerRoute />, "/compose/:id", "/compose/p1");
    await until(() => host.textContent!.includes("14 of 10"));
    expect(host.textContent).toContain("Shortline takes 10 characters, and this post is 14");
    expect(host.textContent).toContain("Social Approver");
    expect(host.textContent).toContain("Waiting for a decision");
    expect(host.textContent).toContain("spring");
  });
});

describe("mentions and recalling", () => {
  it("finds the @name being typed", () => {
    expect(mentionToken("Hello @an", 9)).toEqual({ start: 6, query: "an" });
    expect(mentionToken("mail a@b.c", 10)).toBeNull();
    expect(mentionToken("Hello @an and more", 9)).toEqual({ start: 6, query: "an" });
  });

  it("types @, searches through the account's mentions, inserts what the server says to insert", async () => {
    const withMentions = { rules: { ...rules.rules, mentions: { operation: "people" } } };
    const fake = server([
      ["POST", "/api/v1/SocialAccount/x/actions/rules", withMentions],
      ["POST", "/api/v1/SocialAccount/x/actions/mentions", { mentions: [{ value: "ann", label: "Ann Lee", picture: null, text: "@ann@example.social" }] }],
    ]);
    const host = await draw(<ComposerRoute />, "/compose", "/compose");
    await press(host.querySelector<HTMLInputElement>("input[type=checkbox]")!);
    await until(() => !!field(host, "original-item-0"));
    const area = field(host, "original-item-0") as HTMLTextAreaElement;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(area, "Hi @an");
      area.setSelectionRange(6, 6);
      area.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await until(() => host.textContent!.includes("Ann Lee"));
    expect(fake.seen.find((s) => s.line.endsWith("/mentions"))?.body).toEqual({ query: "an" });
    await press([...host.querySelectorAll<HTMLElement>("button.option")].find((b) => b.textContent!.includes("Ann Lee")));
    expect((field(host, "original-item-0") as HTMLTextAreaElement).value).toBe("Hi @ann@example.social ");
  });

  it("takes an approval request back", async () => {
    const fake = server([
      ["POST", "/api/v1/SocialPost/p1/approval/recall", { ok: true }],
      ["GET", "/api/v1/SocialPost/p1/approval", { id: "r1", status: "pending", steps: [{ label: "Social Approver", state: "open", need: 1, approvers: [] }], decisions: [] }],
      ["GET", "/api/approvals", { to_decide: [], awaiting_others: [] }],
      ["GET", "/api/v1/SocialPost/p1/tags", { tags: [] }],
      ["GET", "/api/v1/SocialPost/p1/links", { links: [] }],
      ["GET", "/api/v1/SocialPost/p1", { id: "p1", version: 3, headline: "Spring", brand: "b1", text: "Spring", status: "In Review", targets: [{ account: "x", kind: "", settings: {} }], items: [], media: [] }],
    ]);
    const host = await draw(<ComposerRoute />, "/compose/:id", "/compose/p1");
    await until(() => !!byText(host, "button", "Take the request back"));
    await press(byText(host, "button", "Take the request back"));
    expect(fake.seen.some((s) => s.line === "POST /api/v1/SocialPost/p1/approval/recall")).toBe(true);
  });
});
