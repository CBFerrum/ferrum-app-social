import { describe, expect, it } from "vitest";
import { before, changeText, lastDays, sparkline, toCsv } from "./period";

describe("the numbers page's arithmetic", () => {
  it("the period asked for and the one before it, of the same length", () => {
    expect(lastDays(7, "2030-06-14")).toEqual({ from: "2030-06-08", to: "2030-06-14" });
    expect(before("2030-06-08", "2030-06-14")).toEqual({ from: "2030-06-01", to: "2030-06-07" });
    expect(before("2030-03-01", "2030-03-31")).toEqual({ from: "2030-01-29", to: "2030-02-28" });
  });
  it("a series as a line, a single value level", () => {
    expect(sparkline([1, 3, 2], 100, 20)).toBe("0,18.0 50,2.0 100,10.0");
    expect(sparkline([5], 100, 20)).toBe("50,10.0");
    expect(sparkline([])).toBe("");
  });
  it("a change in words, none where there was nothing before", () => {
    expect([changeText(12.5), changeText(-3), changeText(null)]).toEqual(["+12.5%", "-3%", ""]);
  });
  it("CSV quotes what it must and guards formulas", () => {
    expect(toCsv([{ name: "a", label: "Name" }, { name: "b" }], [{ a: "x, y", b: "=1+1" }, { a: 'say "hi"', b: 3 }])).toBe("Name,b\n\"x, y\",'=1+1\n\"say \"\"hi\"\"\",3");
  });
});

describe("native posts among the numbers", () => {
  const name = (id: string) => ({ a1: "North Social" })[id] ?? id;
  it("top posts: the report's and the native ones together by the number chosen, native marked", async () => {
    const { mergeTop, nativeByAccount } = await import("./period");
    const report = [{ post: "Spring launch", account: "North Social", sent_at: "2030-06-02T10:00:00Z", permalink: null, impressions: 100, engagements: 20, clicks: 3 }];
    const natives = [{ id: "n1", account: "a1", text: "Posted from the phone", sent_at: "2030-06-03T10:00:00Z", impressions: 900, engagements: 55, clicks: 1, permalink: "https://net.example/p/1" }];
    const top = mergeTop(report, natives, name, "engagements");
    expect(top.map((r) => [r.post, r.native])).toEqual([["Posted from the phone", "n1"], ["Spring launch", undefined]]);
    expect(mergeTop(report, natives, name, "clicks").map((r) => r.post)).toEqual(["Spring launch", "Posted from the phone"]);
    expect(mergeTop(report, natives, name, "engagements", 1)).toHaveLength(1);
    expect(nativeByAccount([...natives, { ...natives[0], id: "n2", engagements: 5 }], name)).toEqual([{ account: "North Social", posts: 2, impressions: 1800, engagements: 60, clicks: 2 }]);
  });
});
