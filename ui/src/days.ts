// The arithmetic of the calendar: which days a month, a week or a day shows, what range of instants that is on the workspace's
// clock (a day that changes the clocks is 23 or 25 hours long, never assumed 24), where posting times fall on those days, and
// where a post dropped on another day lands. Days are `YYYY-MM-DD` labels; instants are RFC 3339; the clock is the one the kit
// shows times in (`toZonedInput` / `fromZonedInput`), except for posting times, which are the workspace's own (src/when.rs).

import { fromZonedInput, toZonedInput } from "@ferrum/client";

export type View = "month" | "week" | "day";

const DAY = 86_400_000;
const utc = (day: string) => Date.parse(`${day}T00:00:00Z`);
const label = (ms: number) => new Date(ms).toISOString().slice(0, 10);

export const addDays = (day: string, n: number) => label(utc(day) + n * DAY);
/** 1 Monday … 7 Sunday. */
export const weekdayOf = (day: string) => new Date(utc(day)).getUTCDay() || 7;
export const dayOf = (iso: string) => toZonedInput(iso).slice(0, 10);
export const timeOf = (iso: string) => toZonedInput(iso).slice(11, 16);

/** The first day of the week containing `day`, given the weekday a week begins on (1 Monday … 7 Sunday). */
export const weekStart = (day: string, firstDay: number) => addDays(day, -((weekdayOf(day) - firstDay + 7) % 7));

/** The days a view shows around `anchor`: the weeks that touch the month, the seven days of the week, or the day. */
export function daysOf(view: View, anchor: string, firstDay = 1): string[] {
  if (view === "day") return [anchor];
  if (view === "week") {
    const start = weekStart(anchor, firstDay);
    return Array.from({ length: 7 }, (_, i) => addDays(start, i));
  }
  const first = `${anchor.slice(0, 7)}-01`;
  const start = weekStart(first, firstDay);
  const next = new Date(Date.UTC(Number(anchor.slice(0, 4)), Number(anchor.slice(5, 7)), 1));
  const last = label(next.getTime() - DAY);
  const weeks = Math.ceil(((utc(last) - utc(start)) / DAY + 1) / 7);
  return Array.from({ length: weeks * 7 }, (_, i) => addDays(start, i));
}

/** The anchor `by` views away: a month, a week or a day. */
export function shift(view: View, anchor: string, by: number): string {
  if (view === "day") return addDays(anchor, by);
  if (view === "week") return addDays(anchor, 7 * by);
  const d = new Date(Date.UTC(Number(anchor.slice(0, 4)), Number(anchor.slice(5, 7)) - 1 + by, 1));
  return label(d.getTime());
}

/** The instants a view covers, as `[from, to)`: the first day's midnight to the day after the last one's, on the shown clock. */
export function rangeOf(days: string[]): { from: string; to: string } {
  const from = fromZonedInput(`${days[0]}T00:00`)!;
  const to = fromZonedInput(`${addDays(days[days.length - 1], 1)}T00:00`)!;
  return { from, to };
}

/** How long a day is on the shown clock, in hours (23 or 25 where the clocks change). */
export const hoursIn = (day: string) => (Date.parse(fromZonedInput(`${addDays(day, 1)}T00:00`)!) - Date.parse(fromZonedInput(`${day}T00:00`)!)) / 3_600_000;

// ── an instant for a wall-clock time in a named zone (the workspace's), as the server finds it ──

function offsetAt(zone: string, ms: number): number {
  const p = new Intl.DateTimeFormat("en-US", { timeZone: zone, hourCycle: "h23", year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" }).formatToParts(new Date(ms));
  const n = (type: string) => Number(p.find((x) => x.type === type)?.value);
  return Date.UTC(n("year"), n("month") - 1, n("day"), n("hour"), n("minute"), n("second")) - Math.floor(ms / 1000) * 1000;
}

/**
 * The instant at which `zone`'s clock reads `day` `hhmm`. Where the clocks go back and it reads so twice, the first;
 * where they go forward and it never does, the first minute after the gap (when.rs `moment`).
 */
export function zonedInstant(day: string, hhmm: string, zone: string | undefined): string {
  const wall = Date.parse(`${day}T${hhmm}:00Z`);
  if (!zone) return fromZonedInput(`${day}T${hhmm}`) ?? new Date(wall).toISOString();
  const reads = (ms: number) => offsetAt(zone, ms) + Math.floor(ms / 1000) * 1000 === wall;
  const found = [offsetAt(zone, wall - DAY), offsetAt(zone, wall + DAY), offsetAt(zone, wall)].map((o) => wall - o).filter(reads);
  if (found.length) return new Date(Math.min(...found)).toISOString();
  for (let m = 1; m <= 180; m++) {
    const later = wall + m * 60_000;
    const hit = [offsetAt(zone, later - DAY), offsetAt(zone, later + DAY)].map((o) => later - o).filter((ms) => offsetAt(zone, ms) + Math.floor(ms / 1000) * 1000 === later);
    if (hit.length) return new Date(Math.min(...hit)).toISOString();
  }
  return new Date(wall).toISOString();
}

// ── posting times ──

export interface PostingTime {
  brand?: string;
  account?: string;
  /** "Every Day", "Monday" … "Sunday". */
  weekday: string;
  time: string;
  active: boolean;
}
export interface Slot {
  at: string;
  day: string;
  brand: string;
  account: string;
}

const NAMES = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/** Whether a posting time falls on a day. */
export const fallsOn = (p: Pick<PostingTime, "weekday">, day: string) => p.weekday === "Every Day" || !p.weekday || NAMES[weekdayOf(day) - 1] === p.weekday;

/**
 * The places posts may go on these days, by the workspace's clock `zone`: an account with times of its own uses only those, a brand's
 * times are for its accounts that have none (when.rs `slots_for`). Places already past `now` are left out.
 */
export function slotsOn(times: PostingTime[], days: string[], zone: string | undefined, now: Date, accounts: { id: string; brand: string }[]): Slot[] {
  const live = times.filter((p) => p.active !== false && /^\d{1,2}:\d{2}$/.test(p.time));
  const own = new Set(live.filter((p) => p.account).map((p) => p.account!));
  const out: Slot[] = [];
  const seen = new Set<string>();
  for (const day of days)
    for (const p of live) {
      if (!fallsOn(p, day)) continue;
      const hhmm = p.time.padStart(5, "0");
      const at = zonedInstant(day, hhmm, zone);
      if (Date.parse(at) <= now.getTime()) continue;
      const mine = p.account ? [p.account] : accounts.filter((a) => a.brand === p.brand && !own.has(a.id)).map((a) => a.id);
      for (const account of mine.length ? mine : [""]) {
        const key = `${at}|${account}`;
        if (seen.has(key)) continue;
        seen.add(key);
        out.push({ at, day: dayOf(at), brand: p.brand ?? "", account });
      }
    }
  return out.sort((a, b) => Date.parse(a.at) - Date.parse(b.at));
}

// ── moving ──

/** Where a post goes when dropped on another day: the same time of day there, on the shown clock. */
export const moveToDay = (iso: string, day: string): string | null => fromZonedInput(`${day}T${timeOf(iso)}`);

/** The day an arrow key leads to from `day` (left/right a day, up/down a week; reversed in a right-to-left page), or null for another key. */
export function arrowDay(day: string, key: string, rtl = false): string | null {
  const side = rtl ? -1 : 1;
  switch (key) {
    case "ArrowRight":
      return addDays(day, side);
    case "ArrowLeft":
      return addDays(day, -side);
    case "ArrowDown":
      return addDays(day, 7);
    case "ArrowUp":
      return addDays(day, -7);
    default:
      return null;
  }
}

/** Posts grouped by the day they go out on the shown clock, each day in time order. */
export function byDay<T extends object>(posts: T[]): Map<string, T[]> {
  const out = new Map<string, T[]>();
  const at = (p: T) => (p as { scheduled_at?: unknown }).scheduled_at;
  for (const p of posts) {
    const when = at(p);
    if (typeof when !== "string") continue;
    const d = dayOf(when);
    out.set(d, [...(out.get(d) ?? []), p]);
  }
  for (const list of out.values()) list.sort((a, b) => Date.parse(String(at(a))) - Date.parse(String(at(b))));
  return out;
}
