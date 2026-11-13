// The calendar (/calendar): posts by month, week or day on the workspace's clock, as chips with time, headline, the accounts'
// pictures and their status; posting times as empty places in the week and the day; a post dragged to another day or place is
// the `reschedule` action (the server's refusal is shown and the chip goes back), with a "move to…" in the quick look as the way
// without dragging; an empty place composes a post at that time; arrows move between days, Enter opens. Calls: SocialPost list
// (scheduled_at between, with brand / account / status / tag filters), SocialPostingTime list, SocialAccount, SocialBrand,
// actions reschedule / copies / add_to_queue / unschedule / duplicate / retry.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { bootstrap, currentLanguage, formatMoment, fromZonedInput, isolate, refusal, t, toZonedInput, todayIso } from "@ferrum/client";
import { Alert, Badge, Button, ButtonLink, Dialog, Field, ListSkeleton, PageHead, PageStack, StatePanel, StatusBadge, toast } from "@ferrum/kit";
import { addDays, arrowDay, byDay, dayOf, daysOf, moveToDay, type PostingTime, rangeOf, shift, type Slot, slotsOn, timeOf, type View } from "./days";
import { type Filter, POST_TONES, record, type Row, rows, run, statusWord, text } from "./data";
import { PostNumbers } from "./PostNumbers";
import { Await, brandName, Face, firstOf, nameOf, useAccounts, useAllowed, useBrands } from "./ui";

const FIELDS = "headline,status,scheduled_at,brand,targets,repeat_every,repeat_unit,repeat_until,repeat_count,repeat_index,repeat_of,waiting_for,failure";
const MOVABLE = ["Draft", "Rejected", "In Review", "Approved"];
const STATUSES = ["Draft", "In Review", "Approved", "Rejected", "Sending", "Published", "Failed"];

/** The weekday a week begins on in the page's language (1 Monday … 7 Sunday); Monday where the browser does not say. */
export function firstWeekday(): number {
  try {
    return (new Intl.Locale(currentLanguage()) as Intl.Locale & { weekInfo?: { firstDay: number } }).weekInfo?.firstDay ?? 1;
  } catch {
    return 1;
  }
}

function useNarrow(): boolean {
  const query = typeof window !== "undefined" && window.matchMedia ? window.matchMedia("(max-width: 720px)") : null;
  const [narrow, setNarrow] = useState(!!query?.matches);
  useEffect(() => {
    if (!query?.addEventListener) return;
    const on = () => setNarrow(query.matches);
    query.addEventListener("change", on);
    return () => query.removeEventListener("change", on);
  }, [query]);
  return narrow;
}

export interface Filters {
  brand: string;
  account: string;
  status: string;
  tag: string;
}

/** The server filters for what the calendar shows between `from` and `to` (exclusive). */
export function filtersFor(from: string, to: string, f: Filters): Filter[] {
  const out: Filter[] = [["scheduled_at", "between", [from, to]]];
  if (f.brand) out.push(["brand", "=", f.brand]);
  if (f.account) out.push(["targets.account", "in", [f.account]]);
  if (f.status) out.push(["status", "in", [f.status]]);
  if (f.tag) out.push(["_tag", "=", f.tag]);
  return out;
}

export interface Placed extends Row {
  _accounts: string[];
}

/** The posts between two instants, each with the accounts it goes to (from its targets; else one small read for each account). */
export async function postsBetween(from: string, to: string, f: Filters, accounts: Row[]): Promise<Placed[]> {
  const filters = filtersFor(from, to, f);
  const found = await rows("SocialPost", filters, { fields: FIELDS, sort: "scheduled_at asc" });
  // The first answer holds the targets when asked for them; one read per account only when no row carries any.
  if (!found.length || found.some((p) => Array.isArray(p.targets))) return found.map((p) => ({ ...p, _accounts: (Array.isArray(p.targets) ? (p.targets as Row[]) : []).map((x) => text(x.account)).filter(Boolean) }));
  const by = new Map<string, string[]>();
  for (const a of accounts.slice(0, 40)) for (const p of await rows("SocialPost", [...filters, ["targets.account", "in", [a.id]]], { fields: "headline" })) by.set(p.id, [...(by.get(p.id) ?? []), a.id]);
  return found.map((p) => ({ ...p, _accounts: by.get(p.id) ?? [] }));
}

const clock = (iso: string) => formatMoment(iso, { hour: "numeric", minute: "2-digit" });
const longDay = (day: string) => formatMoment(zoned(day), { weekday: "long", day: "numeric", month: "long" });
/** Noon of a day on the shown clock, as an instant (a label for the day, safe from clock changes). */
const zoned = (day: string) => fromZonedInput(`${day}T12:00`) ?? `${day}T12:00:00Z`;

/** At most two account marks, then "+n" for the rest (the full list is in the chip's accessible name and the quick look). */
export function Faces({ ids, accounts, max = 2 }: { ids: string[]; accounts: Row[]; max?: number }) {
  const shown = ids.slice(0, max);
  return (
    <span className="cal-faces">
      {shown.map((id) => {
        const a = accounts.find((x) => x.id === id);
        return a ? <Face key={id} name={text(a.account_name)} picture={text(a.picture)} small /> : null;
      })}
      {ids.length > max && <span className="cal-more-faces">+{ids.length - max}</span>}
    </span>
  );
}

export function Chip({ post, accounts, onOpen, onDrag }: { post: Placed; accounts: Row[]; onOpen: (p: Placed) => void; onDrag: (id: string | null) => void }) {
  const status = text(post.status);
  const movable = MOVABLE.includes(status);
  return (
    <button
      type="button"
      className="cal-chip"
      data-tone={POST_TONES[status as keyof typeof POST_TONES] ?? "neutral"}
      data-post={post.id}
      draggable={movable}
      onDragStart={(e) => {
        e.dataTransfer?.setData("text/plain", post.id);
        onDrag(post.id);
      }}
      onDragEnd={() => onDrag(null)}
      onClick={() => onOpen(post)}
      aria-label={t("{headline}, {status}, {at}", { headline: isolate(text(post.headline)), status: statusWord(status), at: formatMoment(text(post.scheduled_at), { dateStyle: "medium", timeStyle: "short" }) })}
    >
      <span className="cal-chip-time">{timeOf(text(post.scheduled_at))}</span>
      <span className="cal-chip-title">{isolate(text(post.headline))}</span>
      {post.repeat_every != null && <span className="cal-repeat" title={t("Repeats")} aria-label={t("Repeats")}>↻</span>}
      <Faces ids={post._accounts} accounts={accounts} />
    </button>
  );
}

/** A network that said "later" leaves a failure that begins so: it is waiting to try again, not failed. */
export const waiting = (failure: string) => failure.startsWith("Waiting to try again");

/** What a repeating post says of its rule, in words. */
export function repeatWords(post: Row): string {
  if (post.repeat_every == null) return "";
  const n = Number(post.repeat_every);
  const unit = text(post.repeat_unit);
  const every = unit === "Weeks" ? t("Repeats every {n} weeks", { n: String(n) }) : unit === "Months" ? t("Repeats every {n} months", { n: String(n) }) : t("Repeats every {n} days", { n: String(n) });
  const end = text(post.repeat_until) ? t(", until {day}", { day: formatMoment(zoned(text(post.repeat_until).slice(0, 10)), { dateStyle: "medium" }) }) : post.repeat_count != null ? t(", {n} times in all", { n: String(post.repeat_count) }) : "";
  return every + end;
}

/** A post looked at without leaving the calendar: its accounts' copies, its state and why, and what may be done with it. */
export function QuickLook({ id, seed, accounts, brands, onClose, onMoved }: { id: string; /** The calendar's own row: shown at once while the record and the copies are read. */ seed?: Placed; accounts: Row[]; brands: Row[]; onClose: () => void; onMoved: (id: string, at: string) => Promise<boolean> }) {
  const client = useQueryClient();
  const post = useQuery({ queryKey: ["social", "post", id], queryFn: () => record("SocialPost", id) });
  const copies = useQuery({
    queryKey: ["social", "copies", id, post.data?.version ?? "seed"],
    queryFn: () => run<{ copies: { account: string; account_name: string; title?: string | null; items: { text: string; length: number; max: number | null }[]; problems: string[] }[] }>("SocialPost", id, "copies").then((r) => r.copies),
    enabled: !!(post.data ?? seed),
    retry: false,
  });
  const [at, setAt] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [refused, setRefused] = useState("");
  const canWrite = useAllowed("SocialPost", "write");
  useEffect(() => {
    if (post.data) setAt(toZonedInput(post.data.scheduled_at));
  }, [post.data]);
  const act = async (name: string, then?: string) => {
    setBusy(name);
    setRefused("");
    try {
      await run("SocialPost", id, name);
      if (then) toast(then);
      await client.invalidateQueries({ queryKey: ["social"] });
      onClose();
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(null);
    }
  };
  const p: Row | undefined = post.data ?? seed;
  const status = text(p?.status);
  return (
    <Dialog
      open
      onClose={onClose}
      wide
      title={isolate(text(p?.headline) || t("Post"))}
      description={p ? `${brandName(brands, p.brand)} · ${formatMoment(text(p.scheduled_at), { dateStyle: "medium", timeStyle: "short" })}` : undefined}
      footer={
        <>
          <Button onClick={onClose}>{t("Close")}</Button>
          <ButtonLink variant="primary" to={`/compose/${id}`}>
            {t("Open")}
          </ButtonLink>
        </>
      }
    >
      <Await q={p ? ({ isPending: false, isError: false, data: p } as never) : post} skeleton={<ListSkeleton rows={3} />}>
        {(row: Row) => (
          <div className="stack">
            <div className="row">
              {seed && <Faces ids={seed._accounts} accounts={accounts} max={6} />}
              <StatusBadge value={status} tones={POST_TONES} label={statusWord(status)} />
              {repeatWords(row) && <Badge>{repeatWords(row)}</Badge>}
              {text(row.repeat_of) && <Link to={`/compose/${text(row.repeat_of)}`}>{t("The first of the repeats")}</Link>}
            </div>
            {text(row.failure) && <Alert tone={waiting(text(row.failure)) ? "warning" : "error"}>{isolate(text(row.failure))}</Alert>}
            {text(row.waiting_for) && <Alert tone="warning">{t("Waiting: {why}", { why: isolate(text(row.waiting_for)) })}</Alert>}
            {text(row.review_note) && <Alert tone="warning">{isolate(text(row.review_note))}</Alert>}
            {refused && <Alert tone="error">{isolate(refused)}</Alert>}
            <Await q={copies} skeleton={<ListSkeleton rows={2} />}>
              {(list) => (
                <ul className="stack" aria-label={t("As each account gets it")}>
                  {list.map((c) => {
                    const a = accounts.find((x) => x.id === c.account);
                    return (
                      <li key={c.account} className="card card-body stack">
                        <div className="row">
                          {a && <Face name={text(a.account_name)} picture={text(a.picture)} />}
                          <strong>{isolate(c.account_name ?? "")}</strong>
                        </div>
                        {c.title?.trim() && <strong>{isolate(c.title)}</strong>}
                        <p className="preview-text">{isolate(c.items[0]?.text ?? "")}</p>
                        {c.problems.map((w, i) => (
                          <p key={i} className="field-error">{isolate(w)}</p>
                        ))}
                      </li>
                    );
                  })}
                </ul>
              )}
            </Await>
            {["Published", "Sending", "Failed"].includes(status) && <PostNumbers post={id} accounts={accounts} />}
            {canWrite && MOVABLE.includes(status) && (
              <div className="row">
                <Field label={t("Move to…")} htmlFor="ql-move">
                  <input id="ql-move" className="input" type="datetime-local" value={at} onChange={(e) => setAt(e.target.value)} />
                </Field>
                <Button
                  loading={busy === "move"}
                  disabled={!at}
                  onClick={async () => {
                    setBusy("move");
                    const iso = fromZonedInput(at);
                    if (iso && (await onMoved(id, iso))) onClose();
                    setBusy(null);
                  }}
                >
                  {t("Move")}
                </Button>
              </div>
            )}
            {canWrite && (
              <div className="row">
                {MOVABLE.includes(status) && <Button loading={busy === "add_to_queue"} onClick={() => void act("add_to_queue", t("Added to the queue"))}>{t("Add to queue")}</Button>}
                {["In Review", "Approved"].includes(status) && <Button loading={busy === "unschedule"} onClick={() => void act("unschedule", t("Back to a draft"))}>{t("Back to draft")}</Button>}
                {status === "Failed" && <Button loading={busy === "retry"} onClick={() => void act("retry", t("Sending again"))}>{t("Retry")}</Button>}
                <Button loading={busy === "duplicate"} onClick={() => void act("duplicate", t("Duplicated as a draft"))}>{t("Duplicate")}</Button>
              </div>
            )}
          </div>
        )}
      </Await>
    </Dialog>
  );
}

export function CalendarRoute() {
  const [params, setParams] = useSearchParams();
  const navigate = useNavigate();
  const client = useQueryClient();
  const today = todayIso();
  const view: View = params.get("v") === "week" ? "week" : params.get("v") === "day" ? "day" : "month";
  const anchor = /^\d{4}-\d{2}-\d{2}$/.test(params.get("d") ?? "") ? params.get("d")! : today;
  const filters: Filters = { brand: params.get("brand") ?? "", account: params.get("account") ?? "", status: params.get("status") ?? "", tag: params.get("tag") ?? "" };
  const first = firstWeekday();
  const days = daysOf(view, anchor, first);
  const range = rangeOf(days);
  const accounts = useAccounts();
  const brands = useBrands();
  const times = useQuery({ queryKey: ["social", "posting-times"], queryFn: () => rows("SocialPostingTime").catch(() => [] as Row[]) });
  const tags = useQuery({ queryKey: ["social", "tags"], queryFn: () => import("@ferrum/client").then(({ api }) => api.get<{ tags: (string | { tag: string })[] }>("/api/tags/SocialPost")).then((r) => r.tags.map((x) => (typeof x === "string" ? x : x.tag))).catch(() => [] as string[]) });
  const posts = useQuery({
    queryKey: ["social", "calendar", range.from, range.to, filters],
    queryFn: () => postsBetween(range.from, range.to, filters, accounts.data ?? []),
    enabled: !!accounts.data,
  });
  const canWrite = useAllowed("SocialPost", "write");
  const narrow = useNarrow();
  const [looking, setLooking] = useState<string | null>(null);
  const [dragged, setDragged] = useState<string | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const [moved, setMoved] = useState<Record<string, string>>({});
  const [refused, setRefused] = useState("");
  const [focus, setFocus] = useState(anchor);
  const grid = useRef<HTMLDivElement>(null);
  const put = (next: Record<string, string>) => setParams((cur) => { const o = Object.fromEntries(cur); for (const [k, v] of Object.entries(next)) v ? (o[k] = v) : delete o[k]; return o; }, { replace: true });

  /** A post to another time: put there at once, and back where it was with the server's words if it is refused. */
  const reschedule = async (id: string, at: string): Promise<boolean> => {
    setRefused("");
    setMoved((m) => ({ ...m, [id]: at }));
    try {
      await run("SocialPost", id, "reschedule", { at });
      await client.invalidateQueries({ queryKey: ["social"] });
      toast(t("Moved to {at}", { at: formatMoment(at, { dateStyle: "medium", timeStyle: "short" }) }));
      return true;
    } catch (e) {
      setRefused(refusal(e));
      return false;
    } finally {
      setMoved(({ [id]: _gone, ...rest }) => rest);
    }
  };
  const drop = (at: string | null) => {
    const id = dragged;
    setDragged(null);
    setOver(null);
    if (id && at) void reschedule(id, at);
  };

  const q = firstOf(accounts, brands);
  const shown: Placed[] = (posts.data ?? []).map((p) => (moved[p.id] ? { ...p, scheduled_at: moved[p.id] } : p));
  const perDay = byDay(shown);
  const zone = bootstrap()?.locale.timezone;
  const slots: Slot[] = view === "month" ? [] : slotsOn((times.data ?? []).map((r) => ({ brand: text(r.brand), account: text(r.account), weekday: text(r.weekday) || "Every Day", time: text(r.time), active: r.active !== false })) as PostingTime[], days, zone, new Date(), (accounts.data ?? []).filter((a) => (!filters.brand || a.brand === filters.brand) && (!filters.account || a.id === filters.account)).map((a) => ({ id: a.id, brand: text(a.brand) })));
  const taken = new Set(shown.map((p) => Date.parse(text(p.scheduled_at))));
  const free = slots.filter((s) => !taken.has(Date.parse(s.at)));
  const rtl = typeof document !== "undefined" && document.documentElement.dir === "rtl";
  const composeAt = (at: string, account = "") => navigate(`/compose?at=${encodeURIComponent(toZonedInput(at))}${account ? `&account=${account}` : ""}`);
  const label = view === "month" ? formatMoment(zoned(`${anchor.slice(0, 7)}-15`), { month: "long", year: "numeric" }) : view === "week" ? `${longDay(days[0])} – ${longDay(days[6])}` : longDay(anchor);
  const weekdays = days.slice(0, 7).map((d) => formatMoment(zoned(d), { weekday: "short" }));

  const byKey = useRef(false);
  useEffect(() => {
    // After an arrow moved the focus to a day, the day's cell takes it (not when the page opens).
    if (!byKey.current) return;
    byKey.current = false;
    grid.current?.querySelector<HTMLElement>(`[data-day="${focus}"] .cal-date, [data-day="${focus}"].cal-col-head`)?.focus();
  }, [focus, days]);
  const onKey = (e: React.KeyboardEvent, day: string) => {
    const next = arrowDay(day, e.key, rtl);
    if (next) {
      e.preventDefault();
      byKey.current = true;
      if (!days.includes(next)) put({ d: next });
      setFocus(next);
    }
  };

  const renderDay = (day: string) => {
    const list = perDay.get(day) ?? [];
    const cellSlots = free.filter((s) => s.day === day);
    const entries = [...list.map((p) => ({ at: Date.parse(text(p.scheduled_at)), post: p, slot: null as Slot | null })), ...cellSlots.map((s) => ({ at: Date.parse(s.at), post: null as Placed | null, slot: s }))].sort((a, b) => a.at - b.at);
    return (
      <div key={day} className="cal-col" data-day={day} data-over={over === day || undefined} onDragOver={(e) => { if (dragged) { e.preventDefault(); setOver(day); } }} onDragLeave={() => setOver((o) => (o === day ? null : o))} onDrop={(e) => { e.preventDefault(); const p = shown.find((x) => x.id === dragged); drop(p ? moveToDay(text(p.scheduled_at), day) : null); }}>
        <button type="button" className="cal-col-head cal-date-head" data-day={day} aria-label={longDay(day)} onClick={() => put({ v: "day", d: day })} onKeyDown={(e) => onKey(e, day)}>
          {longDay(day)}
        </button>
        {entries.length === 0 && <p className="faint">{t("Nothing")}</p>}
        {entries.map((en) =>
          en.post ? (
            <Chip key={en.post.id} post={en.post} accounts={accounts.data ?? []} onOpen={(p) => setLooking(p.id)} onDrag={setDragged} />
          ) : (
            <button
              key={`${en.slot!.at}${en.slot!.account}`}
              type="button"
              className="cal-place"
              data-over={over === en.slot!.at || undefined}
              onClick={() => composeAt(en.slot!.at, en.slot!.account)}
              onDragOver={(e) => { if (dragged) { e.preventDefault(); e.stopPropagation(); setOver(en.slot!.at); } }}
              onDragLeave={() => setOver((o) => (o === en.slot!.at ? null : o))}
              onDrop={(e) => { e.preventDefault(); e.stopPropagation(); drop(en.slot!.at); }}
              aria-label={t("Free place at {at}: write a post for it", { at: formatMoment(en.slot!.at, { dateStyle: "medium", timeStyle: "short" }) })}
            >
              <span className="cal-chip-time">{clock(en.slot!.at)}</span> {t("Free")}
              {en.slot!.account && <Faces ids={[en.slot!.account]} accounts={accounts.data ?? []} />}
            </button>
          ),
        )}
      </div>
    );
  };

  return (
    <PageStack>
      <PageHead
        title={t("Calendar")}
        description={t("Posts by the day and time they go out, on the workspace's clock.")}
        actions={<ButtonLink to="/compose" variant="primary" icon="plus">{t("New post")}</ButtonLink>}
      />
      <div className="row">
        <Button size="sm" icon="left" aria-label={t("Previous")} onClick={() => put({ d: shift(view, anchor, -1) })} />
        <h2 className="section-title" aria-live="polite">{label}</h2>
        <Button size="sm" icon="chevronRight" aria-label={t("Next")} onClick={() => put({ d: shift(view, anchor, 1) })} />
        <Button size="sm" onClick={() => put({ d: "" })}>{t("Today")}</Button>
        <div className="segmented" role="group" aria-label={t("View")}>
          {(["month", "week", "day"] as View[]).map((v) => (
            <button key={v} type="button" aria-pressed={view === v} onClick={() => put({ v: v === "month" ? "" : v })}>
              {v === "month" ? t("Month") : v === "week" ? t("Week") : t("Day")}
            </button>
          ))}
        </div>
      </div>
      <div className="row">
        <Field label={t("Brand")} htmlFor="cal-brand">
          <select id="cal-brand" className="select" value={filters.brand} onChange={(e) => put({ brand: e.target.value, account: "" })}>
            <option value="">{t("All brands")}</option>
            {(brands.data ?? []).map((b) => <option key={b.id} value={b.id}>{isolate(text(b.brand_name))}</option>)}
          </select>
        </Field>
        <Field label={t("Account")} htmlFor="cal-account">
          <select id="cal-account" className="select" value={filters.account} onChange={(e) => put({ account: e.target.value })}>
            <option value="">{t("All accounts")}</option>
            {(accounts.data ?? []).filter((a) => !filters.brand || a.brand === filters.brand).map((a) => <option key={a.id} value={a.id}>{nameOf(a)}</option>)}
          </select>
        </Field>
        <Field label={t("Status")} htmlFor="cal-status">
          <select id="cal-status" className="select" value={filters.status} onChange={(e) => put({ status: e.target.value })}>
            <option value="">{t("Any status")}</option>
            {STATUSES.map((s) => <option key={s} value={s}>{statusWord(s)}</option>)}
          </select>
        </Field>
        <Field label={t("Tag")} htmlFor="cal-tag">
          <select id="cal-tag" className="select" value={filters.tag} onChange={(e) => put({ tag: e.target.value })}>
            <option value="">{t("Any tag")}</option>
            {(tags.data ?? []).map((g) => <option key={g} value={g}>{isolate(g)}</option>)}
          </select>
        </Field>
      </div>
      {refused && <Alert tone="error">{isolate(refused)}</Alert>}
      <Await q={firstOf(q, posts)} skeleton={<ListSkeleton rows={6} />}>
        {() => {
          if (!shown.length && !free.length && (filters.brand || filters.account || filters.status || filters.tag))
            return <StatePanel kind="no-matches" onClear={() => put({ brand: "", account: "", status: "", tag: "" })}>{t("No post matches these filters in this period.")}</StatePanel>;
          if (narrow)
            return (
              <ul className="agenda-list" aria-label={label}>
                {days.filter((d) => view !== "month" || d.startsWith(anchor.slice(0, 7))).map((d) => ({ d, list: perDay.get(d) ?? [] })).filter((x) => x.list.length || view !== "month").map(({ d, list }) => (
                  <li key={d} className="stack">
                    <strong>{longDay(d)}</strong>
                    {list.length === 0 && <span className="faint">{t("Nothing")}</span>}
                    {list.map((p) => <Chip key={p.id} post={p} accounts={accounts.data ?? []} onOpen={(x) => setLooking(x.id)} onDrag={setDragged} />)}
                  </li>
                ))}
              </ul>
            );
          return view === "month" ? (
            <div ref={grid} className="cal-grid" role="grid" aria-label={label}>
              <div className="cal-row" role="row">{weekdays.map((d, i) => <div key={i} className="cal-weekday" role="columnheader">{isolate(d)}</div>)}</div>
              {Array.from({ length: days.length / 7 }, (_, w) => (
                <div key={w} className="cal-row" role="row">
                  {days.slice(w * 7, w * 7 + 7).map((day) => {
                    const list = perDay.get(day) ?? [];
                    return (
                      <div key={day} role="gridcell" className="cal-day" data-day={day} data-other={day.slice(0, 7) !== anchor.slice(0, 7)} data-today={day === today} data-over={over === day || undefined}
                        onDragOver={(e) => { if (dragged) { e.preventDefault(); setOver(day); } }} onDragLeave={() => setOver((o) => (o === day ? null : o))}
                        onDrop={(e) => { e.preventDefault(); const p = shown.find((x) => x.id === dragged); drop(p ? moveToDay(text(p.scheduled_at), day) : null); }}>
                        <button type="button" className="cal-date" tabIndex={day === focus || (!days.includes(focus) && day === days[0]) ? 0 : -1} aria-label={t("{day}: {n} posts. Enter opens the day.", { day: longDay(day), n: String(list.length) })}
                          onClick={() => { setFocus(day); put({ v: "day", d: day }); }} onKeyDown={(e) => onKey(e, day)}>
                          {Number(day.slice(8))}
                          {list.length > 0 && <span className="cal-count">{list.length}</span>}
                        </button>
                        <ul className="cal-items">
                          {list.slice(0, 3).map((p) => <li key={p.id}><Chip post={p} accounts={accounts.data ?? []} onOpen={(x) => setLooking(x.id)} onDrag={setDragged} /></li>)}
                          {list.length > 3 && <li><button type="button" className="cal-more" onClick={() => put({ v: "day", d: day })}>{t("{n} more", { n: String(list.length - 3) })}</button></li>}
                        </ul>
                        {canWrite && list.length === 0 && <button type="button" className="cal-add" aria-label={t("Write a post for {day}", { day: longDay(day) })} onClick={() => composeAt(zonedAt(day))}>+</button>}
                      </div>
                    );
                  })}
                </div>
              ))}
            </div>
          ) : (
            <div ref={grid} className="cal-cols" data-days={days.length}>
              {days.map((d) => renderDay(d))}
            </div>
          );
        }}
      </Await>
      {looking && <QuickLook id={looking} seed={shown.find((x) => x.id === looking)} accounts={accounts.data ?? []} brands={brands.data ?? []} onClose={() => setLooking(null)} onMoved={reschedule} />}
      <p className="muted"><Link to="/queue">{t("Posting times")}</Link></p>
    </PageStack>
  );
}

/** A day's first sensible time for a new post: nine in the morning on the shown clock. */
const zonedAt = (day: string) => fromZonedInput(`${day}T09:00`) ?? `${day}T09:00:00Z`;
export { addDays, dayOf, timeOf };
