import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { resetBootstrap, setDisplayZone, setTimeZone } from "@ferrum/client";
import { addDays, arrowDay, byDay, daysOf, dayOf, hoursIn, moveToDay, rangeOf, shift, slotsOn, weekStart, zonedInstant } from "./days";

beforeEach(() => setTimeZone("America/New_York"));
afterEach(() => {
  resetBootstrap();
  setDisplayZone(undefined);
});

describe("which days a view shows", () => {
  it("a month is whole weeks, from the weekday a week begins on", () => {
    const monday = daysOf("month", "2026-11-14", 1);
    expect(monday[0]).toBe("2026-10-26");
    expect(monday.length % 7).toBe(0);
    expect(monday[monday.length - 1]).toBe("2026-12-06");
    expect(daysOf("month", "2026-11-14", 7)[0]).toBe("2026-11-01");
  });
  it("a week, a day, and moving between them", () => {
    expect(daysOf("week", "2026-11-14", 1)).toEqual(["2026-11-09", "2026-11-10", "2026-11-11", "2026-11-12", "2026-11-13", "2026-11-14", "2026-11-15"]);
    expect(daysOf("day", "2026-11-14")).toEqual(["2026-11-14"]);
    expect(shift("month", "2026-12-14", 1)).toBe("2027-01-01");
    expect(shift("month", "2026-01-31", -1)).toBe("2025-12-01");
    expect(shift("week", "2026-11-14", -1)).toBe("2026-11-07");
    expect(weekStart("2026-11-15", 1)).toBe("2026-11-09");
  });
});

describe("a day where the clocks change", () => {
  it("is 23 hours long in spring and 25 in autumn, so the range follows the zone, not 24 hours a day", () => {
    expect(hoursIn("2026-03-08")).toBe(23);
    expect(hoursIn("2026-11-01")).toBe(25);
    expect(hoursIn("2026-11-02")).toBe(24);
    const range = rangeOf(["2026-03-07", "2026-03-08", "2026-03-09"]);
    expect(range).toEqual({ from: "2026-03-07T05:00:00.000Z", to: "2026-03-10T04:00:00.000Z" });
  });
  it("a post's day is the day on the shown clock, not UTC", () => {
    expect(dayOf("2026-11-14T03:30:00Z")).toBe("2026-11-13");
    expect(moveToDay("2026-11-14T03:30:00Z", "2026-11-20")).toBe("2026-11-21T03:30:00.000Z");
    // the same time of day across the change keeps the wall clock, not the UTC hour
    expect(moveToDay("2026-03-06T14:00:00Z", "2026-03-09")).toBe("2026-03-09T13:00:00.000Z");
  });
  it("a time that never happens moves to after the gap, one that happens twice is the first", () => {
    expect(zonedInstant("2026-03-08", "02:30", "America/New_York")).toBe("2026-03-08T07:00:00.000Z");
    expect(zonedInstant("2026-11-01", "01:30", "America/New_York")).toBe("2026-11-01T05:30:00.000Z");
    expect(zonedInstant("2026-11-14", "09:00", "America/New_York")).toBe("2026-11-14T14:00:00.000Z");
    expect(zonedInstant("2026-11-14", "09:00", "Asia/Kolkata")).toBe("2026-11-14T03:30:00.000Z");
  });
});

describe("posting times as places", () => {
  const accounts = [{ id: "a", brand: "b" }, { id: "c", brand: "b" }];
  const now = new Date("2026-11-01T00:00:00Z");
  it("a brand's times serve its accounts without times of their own; an account with its own uses only those", () => {
    const times = [
      { brand: "b", weekday: "Every Day", time: "09:00", active: true },
      { brand: "b", account: "c", weekday: "Monday", time: "18:30", active: true },
      { brand: "b", weekday: "Every Day", time: "12:00", active: false },
    ];
    const slots = slotsOn(times, ["2026-11-09", "2026-11-10"], "America/New_York", now, accounts);
    expect(slots.map((s) => [s.day, s.account, s.at.slice(11, 16)])).toEqual([
      ["2026-11-09", "a", "14:00"],
      ["2026-11-09", "c", "23:30"],
      ["2026-11-10", "a", "14:00"],
    ]);
  });
  it("places that are past are not offered", () => {
    expect(slotsOn([{ brand: "b", weekday: "Every Day", time: "09:00", active: true }], ["2026-11-09"], "America/New_York", new Date("2026-11-09T20:00:00Z"), accounts)).toEqual([]);
  });
  it("the workspace's clock, not the shown one, places a time", () => {
    setDisplayZone("Asia/Kolkata");
    const [slot] = slotsOn([{ brand: "b", weekday: "Every Day", time: "09:00", active: true }], ["2026-11-09"], "America/New_York", now, accounts);
    expect(slot.at).toBe("2026-11-09T14:00:00.000Z");
    expect(slot.day).toBe("2026-11-09");
  });
});

describe("the keyboard and grouping", () => {
  it("arrows lead from day to day", () => {
    expect(arrowDay("2026-11-14", "ArrowRight")).toBe("2026-11-15");
    expect(arrowDay("2026-11-14", "ArrowRight", true)).toBe("2026-11-13");
    expect(arrowDay("2026-11-14", "ArrowDown")).toBe("2026-11-21");
    expect(arrowDay("2026-11-14", "ArrowUp")).toBe("2026-11-07");
    expect(arrowDay("2026-11-14", "Enter")).toBeNull();
    expect(addDays("2026-02-28", 1)).toBe("2026-03-01");
  });
  it("posts by day in time order, drafts without a time left out", () => {
    const m = byDay([{ id: 2, scheduled_at: "2026-11-14T18:00:00Z" }, { id: 1, scheduled_at: "2026-11-14T15:00:00Z" }, { id: 3, scheduled_at: null }, { id: 4, scheduled_at: "2026-11-15T10:00:00Z" }]);
    expect([...m.keys()]).toEqual(["2026-11-14", "2026-11-15"]);
    expect(m.get("2026-11-14")!.map((p) => p.id)).toEqual([1, 2]);
  });
});
