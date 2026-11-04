import { describe, expect, it } from "vitest";
import { byDay, rate, totals } from "./Home";

describe("social's home", () => {
  it("posts by day", () => {
    const days = byDay([
      { id: "2", scheduled_at: "2026-11-06T09:00:00Z" },
      { id: "1", scheduled_at: "2026-11-05T15:00:00Z" },
      { id: "3", scheduled_at: "2026-11-06T18:00:00Z" },
    ]);
    expect(days.map(([d, p]) => [d, p.map((x) => x.id)])).toEqual([
      ["2026-11-05", ["1"]],
      ["2026-11-06", ["2", "3"]],
    ]);
  });

  it("what posts did, by brand", () => {
    const accounts = [
      { id: "a", brand: "nw" },
      { id: "b", brand: "nw" },
      { id: "c", brand: "co" },
    ];
    const sums = totals(
      [
        { id: "1", account: "a", post: "p", impressions: 100, engagements: 5 },
        { id: "2", account: "b", post: "p", impressions: 50, engagements: 1 },
        { id: "3", account: "c", post: null, impressions: 10, engagements: 0 },
      ],
      accounts,
    );
    expect([sums.get("nw")?.posts.size, sums.get("nw")?.impressions, sums.get("co")?.posts.size]).toEqual([1, 150, 0]);
    expect([rate(150, 6), rate(0, 0)]).toEqual(["4.0%", "–"]);
  });
});
