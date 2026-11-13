import { describe, expect, it } from "vitest";
import { answers, applies, clusters, effective, fill, finalText, itemLength, length, NO_RULES, problems, type Rules, settingProblems } from "./model";

const rules = (over: Partial<Rules>): Rules => ({ ...NO_RULES, ...over });
const pictures = { kinds: ["image"], required: false, mixes: [], other: {}, alt: true, alt_required: true, thumbnail: false, max: 4, image: { types: [], max_bytes: 1000 } } as Rules["media"];

describe("how a network counts", () => {
  it("counts as rules.rs does", () => {
    const c = (count: "characters" | "graphemes" | "utf16" | "bytes" | "weighted", link_counts?: number) => ({ count, link_counts });
    expect(length(c("characters"), "héllo")).toBe(5);
    expect(length(c("bytes"), "héllo")).toBe(6);
    expect(length(c("utf16"), "a😀")).toBe(3);
    expect(length(c("characters"), "a😀")).toBe(2);
    expect(length(c("graphemes"), "👨‍👩‍👧🇮🇳👍🏽é")).toBe(4);
    expect(clusters("🇮🇳🇫🇷")).toHaveLength(2);
    expect(length(c("weighted", 23), "hi 日本 😀")).toBe(3 + 4 + 1 + 2);
    expect(length(c("weighted", 23), "see https://example.org/a/very/long/path ok")).toBe(4 + 23 + 3);
    expect(length(c("characters"), "see https://e.org")).toBe(17);
  });

  it("a link adds a space and its length", () => {
    expect(itemLength({ count: "characters" }, "Hi", "https://e.org")).toBe(2 + 13 + 1);
    expect(itemLength({ count: "weighted", link_counts: 23 }, "Hi", "https://example.org/long/path")).toBe(2 + 23 + 1);
    expect(itemLength({ count: "characters" }, "Hi", "  ")).toBe(2);
  });
});

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
  it("says what is needed", () => {
    expect(settingProblems(r, "pin", {}).map((p) => p.why)).toEqual(["Board is needed"]);
    expect(settingProblems(r, "pin", { board: "zzz" }).map((p) => p.why)).toEqual(["Board is one of the choices"]);
    expect(settingProblems(r, "pin", { board: "a" })).toEqual([]);
  });
});

describe("what is final", () => {
  it("fills variables and names the first with no value", () => {
    expect(fill("Hi {{ name }}, {{city}}", { name: "Ann", city: "Pune" })).toEqual({ text: "Hi Ann, Pune", missing: null });
    expect(fill("Hi {{who}}", {}).missing).toBe("who");
  });
  it("adds the signature once", () => {
    expect(finalText("Hello", "-- Ann", {}).text).toBe("Hello\n\n-- Ann");
    expect(finalText("Hello\n\n-- Ann", "-- Ann", {}).text).toBe("Hello\n\n-- Ann");
    expect(finalText("Hello {{x}}", null, { x: "1" }).text).toBe("Hello 1");
  });
});

describe("the live checks", () => {
  const short = rules({ name: "Short", text: { count: "characters", max: 10 }, follow_ups: { as: "thread", media: false }, media: pictures, first_comment: false });
  const copy = (text: string, extra: Partial<{ items: { text: string; link: string; media: { kind: string; alt?: string; bytes?: number }[] }[]; firstComment: string }> = {}) => ({
    kind: null,
    settings: {},
    firstComment: extra.firstComment ?? "",
    items: extra.items ?? [{ text, link: "", media: [] }],
  });
  it("a text over its most", () => {
    expect(problems(short, copy("a".repeat(11)), "Short")).toEqual(["Short takes 10 characters, and this post is 11"]);
    expect(problems(short, copy("fine"), "Short")).toEqual([]);
  });
  it("follow-ups: not offered, media after the post, a first comment", () => {
    const none = rules({ name: "Plain", follow_ups: { as: "none", media: false } });
    expect(problems(none, copy("a", { items: [{ text: "a", link: "", media: [] }, { text: "b", link: "", media: [] }] }), "Plain")).toEqual(["Plain takes nothing after the post"]);
    expect(problems(short, copy("a", { items: [{ text: "a", link: "", media: [] }, { text: "b", link: "", media: [{ kind: "image", alt: "x" }] }] }), "Short")).toContain("Short takes no media after the post");
    expect(problems(short, copy("a", { firstComment: "first" }), "Short")).toEqual(["Short takes no first comment"]);
  });
  it("media: a kind not taken, too big, no description, too many", () => {
    const withMedia = (media: { kind: string; alt?: string; bytes?: number }[]) => copy("a", { items: [{ text: "a", link: "", media }] });
    expect(problems(short, withMedia([{ kind: "video", alt: "x" }]), "Short")).toEqual(["Short takes no video, only pictures"]);
    expect(problems(short, withMedia([{ kind: "image", alt: "x", bytes: 2000 }]), "Short")).toEqual(["Short takes pictures of at most 1 KB, and this one is 2 KB"]);
    expect(problems(short, withMedia([{ kind: "image" }]), "Short")).toEqual(["Short needs a description of each of its media"]);
    expect(problems(short, withMedia(Array.from({ length: 5 }, () => ({ kind: "image", alt: "x" }))), "Short")).toEqual(["Short takes at most 4 media in one item, and this one has 5"]);
  });
  it("a kind has rules of its own", () => {
    const r = rules({ text: { count: "characters", max: 5 }, kinds: [{ name: "post" }, { name: "article", text: { count: "characters", max: 50 } }] });
    expect(effective(r, "article").text.max).toBe(50);
    expect(effective(r, null).text.max).toBe(5);
  });
});
