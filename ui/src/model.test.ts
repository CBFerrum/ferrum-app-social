import { describe, expect, it } from "vitest";
import { answers, applies, effective, NO_RULES, type Rules } from "./model";

const rules = (over: Partial<Rules>): Rules => ({ ...NO_RULES, ...over });

describe("the settings of a network", () => {
  const r = rules({
    kinds: [{ name: "post" }, { name: "pin" }],
    settings: [
      { key: "board", label: "Board", type: "select", options: ["a", { value: "b", label: "B" }], required: true, when: { kind: ["pin"] } },
      { key: "link_text", label: "Link text", type: "text", options: [], required: false, when: { board: "b" } },
      { key: "notify", label: "Notify", type: "bool", options: [], required: false, default: true, when: {} },
    ],
  });
  it("asks only what applies: by kind, and by another answer", () => {
    expect(r.settings.filter((s) => applies(s, "post", {})).map((s) => s.key)).toEqual(["notify"]);
    expect(r.settings.filter((s) => applies(s, "pin", {})).map((s) => s.key)).toEqual(["board", "notify"]);
    expect(r.settings.filter((s) => applies(s, "pin", { board: "b" })).map((s) => s.key)).toEqual(["board", "link_text", "notify"]);
  });
  it("drops answers to what does not apply and fills defaults", () => {
    expect(answers(r, "post", { board: "a", notify: false })).toEqual({ notify: false });
    expect(answers(r, "pin", { board: "a" })).toEqual({ board: "a", notify: true });
  });
});

describe("a kind of post", () => {
  it("has rules of its own where it says so, the network's where it does not", () => {
    const r = rules({ text: { count: "characters", max: 5, fold: 3 }, kinds: [{ name: "post" }, { name: "article", text: { count: "characters", max: 50 } }] });
    expect(effective(r, "article").text.max).toBe(50);
    expect(effective(r, null).text.max).toBe(5);
    expect(effective(r, "nope").kind?.name).toBe("post");
  });
});
