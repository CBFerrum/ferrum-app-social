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
