// @vitest-environment jsdom
// The home as it is drawn: its queues, with what waits for a decision (the platform's approvals as well as the status), posts
// approved but waiting for an account, accounts to reconnect, drafts without a time, and the ways to the other screens.

import { afterEach, describe, expect, it, vi } from "vitest";
import { Home } from "./Home";
import { draw, fakeServer } from "./testkit";

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const posts = (url: URL) => {
  const f = url.searchParams.get("filters") ?? "";
  if (f.includes("In Review")) return { rows: [{ id: "r1", headline: "Needs a look", author: "ann@example.org", status: "In Review" }] };
  if (f.includes("Failed")) return { rows: [] };
  if (f.includes("Sending")) return { rows: [{ id: "c1", headline: "Goes tomorrow", status: "Approved", scheduled_at: "2030-06-12T09:00:00Z" }] };
  if (f.includes("Draft")) return { rows: [{ id: "d1", headline: "No time yet", status: "Draft" }] };
  if (f.includes("Approved")) return { rows: [{ id: "w1", headline: "Held back", status: "Approved", waiting_for: "North Pins: the provider no longer accepts the sign-in" }, { id: "c1", headline: "Goes tomorrow", status: "Approved", waiting_for: null }] };
  return { rows: [] };
};

describe("social's home", () => {
  it("shows what waits, what is held back and why, and the accounts to reconnect, with links to the other screens", async () => {
    fakeServer([
      ["GET", "/api/approvals", { to_decide: [{ entity: "SocialPost", record: "r2", title: "Asked of you", requester_name: "Bo" }], awaiting_others: [] }],
      ["GET", "/api/v1/SocialPost", posts],
      ["GET", "/api/v1/SocialBrand", { rows: [{ id: "b1", brand_name: "Northwind" }] }],
      ["GET", "/api/v1/SocialAccount", { rows: [{ id: "a1", account_name: "North Pins", brand: "b1", state: "Needs Reconnecting", state_note: "connect the account again", active: true }, { id: "a2", account_name: "Fine", brand: "b1", state: "Connected", active: true }] }],
      ["GET", "/api/v1/SocialMessage", { rows: [] }],
      ["GET", "/api/v1/SocialPublication", { rows: [] }],
    ]);
    const host = await draw(<Home />, "/", "/");
    const text = host.textContent!;
    expect(text).toContain("Needs a look");
    expect(text).toContain("Asked of you");
    expect(text).toContain("Yours to decide");
    expect(text).toContain("Held back");
    expect(text).toContain("North Pins: the provider no longer accepts the sign-in");
    expect(text).not.toContain("Fine");
    expect(text).toContain("North Pins");
    expect(text).toContain("No time yet");
    const links = [...host.querySelectorAll("a")].map((a) => a.getAttribute("href"));
    for (const to of ["/compose", "/calendar", "/queue", "/accounts"]) expect(links).toContain(to);
  });

  it("says plainly when a read is refused", async () => {
    fakeServer([["GET", "/api/v1/", { error: "forbidden", message: "You do not have permission to do that." }, 403]]);
    const host = await draw(<Home />, "/", "/");
    expect(host.textContent).toMatch(/permission|access/i);
  });
});
