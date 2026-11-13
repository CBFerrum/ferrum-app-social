import { describe, expect, it } from "vitest";
import { copyOf, customise, dropVersion, fromRecord, guessKind, missingVariable, removeAccount, toRecord } from "./draft";
import type { Row } from "./data";

// The record's shape is docs/interface-notes.md section 2 (the full example there).
const record = {
  id: "p1",
  headline: "Spring",
  brand: "b1",
  text: "Spring is here",
  link: "",
  add_signature: true,
  scheduled_at: null,
  targets: [
    { account: "x", kind: "", settings: {} },
    { account: "pins", kind: "pin", settings: { board: "b-1" }, text: "Spring, pinned" },
  ],
  items: [
    { text: "Two of three", delay_minutes: 0 },
    { text: "Three of three", delay_minutes: 30 },
    { account: "pins", text: "pins' own follow-up", delay_minutes: 5 },
  ],
  media: [
    { item: 1, file: "/files/a", alt: "A cat" },
    { account: "pins", item: 1, file: "/files/b", alt: "A cat", width: 1000, height: 1500 },
    { item: 2, file: "/files/c", alt: "" },
  ],
} as unknown as Row;

describe("a record as a draft and back", () => {
  it("lays media on the items they name, for the original and for a version", () => {
    const d = fromRecord(record);
    expect(d.original.items.map((i) => i.text)).toEqual(["Spring is here", "Two of three", "Three of three"]);
    expect(d.original.items.map((i) => i.media.map((m) => m.file))).toEqual([["/files/a"], ["/files/c"], []]);
    expect(Object.keys(d.versions)).toEqual(["pins"]);
    expect(d.versions.pins.items.map((i) => i.text)).toEqual(["Spring, pinned", "pins' own follow-up"]);
    expect(d.versions.pins.items[0].media[0]).toMatchObject({ file: "/files/b", width: 1000, height: 1500 });
    expect(d.targets[1]).toEqual({ account: "pins", kind: "pin", settings: { board: "b-1" } });
  });

  it("writes it back with each version's rows naming its account, the original's none", () => {
    const body = toRecord(fromRecord(record), "b1");
    expect(body.items).toEqual([
      { text: "Two of three", link: null, delay_minutes: 0 },
      { text: "Three of three", link: null, delay_minutes: 30 },
      { account: "pins", text: "pins' own follow-up", link: null, delay_minutes: 5 },
    ]);
    const media = body.media as Record<string, unknown>[];
    expect(media.filter((m) => !m.account).map((m) => [m.item, m.file])).toEqual([[1, "/files/a"], [2, "/files/c"]]);
    expect(media.find((m) => m.account === "pins")).toMatchObject({ item: 1, file: "/files/b", width: 1000, height: 1500 });
    const targets = body.targets as Record<string, unknown>[];
    expect(targets[0]).toMatchObject({ account: "x", text: "" });
    expect(targets[1]).toMatchObject({ account: "pins", kind: "pin", text: "Spring, pinned" });
    expect(body.image).toBeNull();
  });

  it("0.7.0's one picture becomes the first medium, and is no longer sent as the image", () => {
    const d = fromRecord({ id: "q", text: "x", image: "/files/old", image_alt: "Old", targets: [{ account: "x" }] } as unknown as Row);
    expect(d.original.items[0].media[0]).toMatchObject({ file: "/files/old", alt: "Old" });
    const body = toRecord(d, "b1");
    expect(body.image).toBeNull();
    expect((body.media as unknown[]).length).toBe(1);
  });

  it("customising copies the original; going back drops the version and its rows", () => {
    let d = fromRecord({ ...record, targets: [{ account: "x" }], items: [], media: [{ item: 1, file: "/files/a", alt: "A" }] } as unknown as Row);
    d = customise(d, "x");
    expect(d.versions.x.items[0].text).toBe("Spring is here");
    expect(d.versions.x.items[0].media[0].file).toBe("/files/a");
    d.versions.x.items[0].text = "Its own";
    const body = toRecord(d, "b1");
    expect((body.targets as Record<string, unknown>[])[0]).toMatchObject({ text: "Its own" });
    expect((body.media as Record<string, unknown>[]).filter((m) => m.account === "x")).toHaveLength(1);
    const back = toRecord(dropVersion(d, "x"), "b1");
    expect((back.targets as Record<string, unknown>[])[0]).toMatchObject({ text: "" });
    expect((back.media as Record<string, unknown>[]).some((m) => m.account)).toBe(false);
  });

  it("an account taken off the post takes its version with it", () => {
    const d = removeAccount(fromRecord(record), "pins");
    expect(d.targets.map((x) => x.account)).toEqual(["x"]);
    expect((toRecord(d, "b1").items as unknown[]).length).toBe(2);
  });

  it("repeat and time are written on the workspace's clock", () => {
    const d = fromRecord(record);
    Object.assign(d, { at: "2026-11-20T09:30", repeatEvery: "2", repeatUnit: "Weeks", repeatCount: "4" });
    const body = toRecord(d, "b1");
    expect(body).toMatchObject({ repeat_every: 2, repeat_unit: "Weeks", repeat_count: 4, repeat_until: null });
    expect(typeof body.scheduled_at).toBe("string");
    expect(toRecord({ ...d, repeatEvery: "" }, "b1")).toMatchObject({ repeat_every: null, repeat_count: null });
  });

  it("answers to settings that do not apply are not sent", () => {
    const d = fromRecord(record);
    d.targets[1].settings = { board: "b-1", stale: "x" };
    const rules = { pins: { settings: [{ key: "board", label: "Board", type: "select", options: [], required: false, when: {} }], kinds: [], text: {}, media: {}, follow_ups: {} } as never };
    expect((toRecord(d, "b1", rules).targets as { settings: unknown }[])[1].settings).toEqual({ board: "b-1" });
  });
});

describe("what an account will get", () => {
  it("its version, else the original, with the signature once and the variables filled", () => {
    const d = fromRecord(record);
    const x = copyOf(d, "x", "-- Ann", { who: "Ann" });
    expect(x.items.map((i) => i.text)).toEqual(["Spring is here\n\n-- Ann", "Two of three", "Three of three"]);
    const pins = copyOf(d, "pins", null, {});
    expect(pins.items.map((i) => i.text)).toEqual(["Spring, pinned", "pins' own follow-up"]);
    expect(pins.kind).toBe("pin");
    d.original.items[0].text = "Hi {{who}}";
    expect(missingVariable(d, "x", {})).toBe("who");
    expect(missingVariable(d, "x", { who: "A" })).toBeNull();
  });
  it("a medium's kind is the stored one, else the file's", () => {
    expect(guessKind("image/png", "/files/a")).toBe("image");
    expect(guessKind(undefined, "/files/a.mp4")).toBe("video");
    expect(guessKind("application/pdf", "/files/a")).toBe("document");
  });
});
