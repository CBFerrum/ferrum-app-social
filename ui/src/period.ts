// The pure parts of the numbers page: the period asked for and the one before it, a series as the points of a small line,
// and a report's rows as CSV.

import { todayIso } from "@ferrum/client";

const DAY = 86_400_000;
const utc = (day: string) => Date.parse(`${day}T00:00:00Z`);
const label = (ms: number) => new Date(ms).toISOString().slice(0, 10);

/** The last `days` days up to and including `end`. */
export const lastDays = (days: number, end = todayIso()) => ({ from: label(utc(end) - (days - 1) * DAY), to: end });

/** The period of the same length just before `from`–`to` (what the server compares with). */
export function before(from: string, to: string): { from: string; to: string } {
  const days = Math.round((utc(to) - utc(from)) / DAY) + 1;
  return { from: label(utc(from) - days * DAY), to: label(utc(from) - DAY) };
}

/** A series as the `points` of a polyline in a `w` by `h` box, the lowest value at the bottom; a single value is a level line. */
export function sparkline(values: number[], w = 120, h = 28): string {
  if (!values.length) return "";
  const lo = Math.min(...values);
  const hi = Math.max(...values);
  const y = (v: number) => (hi === lo ? h / 2 : h - 2 - ((v - lo) / (hi - lo)) * (h - 4));
  return values.map((v, i) => `${values.length === 1 ? w / 2 : (i / (values.length - 1)) * w},${y(v).toFixed(1)}`).join(" ");
}

/** The change in words: a signed percentage, or nothing where the period before had nothing to compare with. */
export const changeText = (pct: unknown): string => (typeof pct === "number" ? `${pct > 0 ? "+" : ""}${pct}%` : "");

/** CSV of a report's raw values, guarded against a spreadsheet taking a cell for a formula. */
export function toCsv(columns: { name: string; label?: string }[], rows: Record<string, unknown>[]): string {
  const cell = (v: unknown) => {
    let s = v == null ? "" : String(v);
    if (/^[=+\-@\t\r]/.test(s)) s = `'${s}`;
    return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  return [columns.map((c) => cell(c.label ?? c.name)).join(","), ...rows.map((r) => columns.map((c) => cell(r[c.name])).join(","))].join("\n");
}

export interface TopRow {
  post: string;
  account: string;
  sent_at: string;
  permalink: string | null;
  impressions: number;
  engagements: number;
  clicks: number;
  /** A publication made on the network itself and read back: it has no post of ours (its `id` is the publication's). */
  native?: string;
}

/** The report's top posts and the native posts of the period together, by the number chosen, the most first; a tie goes to the later. */
export function mergeTop(report: Record<string, unknown>[], natives: Record<string, unknown>[], accountName: (id: string) => string, by: "engagements" | "impressions" | "clicks", limit = 10): TopRow[] {
  const n = (v: unknown) => (v == null ? 0 : Number(v));
  const fromReport: TopRow[] = report.map((r) => ({ post: String(r.post ?? ""), account: String(r.account ?? ""), sent_at: String(r.sent_at ?? ""), permalink: (r.permalink as string) || null, impressions: n(r.impressions), engagements: n(r.engagements), clicks: n(r.clicks) }));
  const fromNative: TopRow[] = natives.map((p) => ({ post: String(p.text ?? "").slice(0, 100), account: accountName(String(p.account)), sent_at: String(p.sent_at ?? ""), permalink: (p.permalink as string) || null, impressions: n(p.impressions), engagements: n(p.engagements), clicks: n(p.clicks), native: String(p.id) }));
  return [...fromReport, ...fromNative].sort((a, b) => b[by] - a[by] || b.sent_at.localeCompare(a.sent_at)).slice(0, limit);
}

/** Native posts added up by account. */
export function nativeByAccount(natives: Record<string, unknown>[], accountName: (id: string) => string): { account: string; posts: number; impressions: number; engagements: number; clicks: number }[] {
  const out = new Map<string, { account: string; posts: number; impressions: number; engagements: number; clicks: number }>();
  for (const p of natives) {
    const name = accountName(String(p.account));
    const row = out.get(name) ?? { account: name, posts: 0, impressions: 0, engagements: 0, clicks: 0 };
    row.posts += 1;
    row.impressions += Number(p.impressions ?? 0);
    row.engagements += Number(p.engagements ?? 0);
    row.clicks += Number(p.clicks ?? 0);
    out.set(name, row);
  }
  return [...out.values()].sort((a, b) => b.engagements - a.engagements || a.account.localeCompare(b.account));
}
