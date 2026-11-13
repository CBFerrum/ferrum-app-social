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
