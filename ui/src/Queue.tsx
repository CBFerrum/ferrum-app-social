// Posting times (/queue): the weekly times of a brand or of one account (add, remove, switch off), the next places those
// times make and what is in each, moving a post to another place (the `reschedule` action, by a choice or by dragging it onto
// the place), and drafts to put into the queue (the `add_to_queue` action shows the time it took). An account with times of
// its own uses only those; the brand's serve the rest (src/when.rs).

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link, useSearchParams } from "react-router";
import { bootstrap, formatMoment, isolate, refusal, t } from "@ferrum/client";
import { Alert, Badge, Button, ButtonLink, confirmDialog, Field, ListSkeleton, PageHead, PageStack, StatePanel, StatusBadge, toast } from "@ferrum/kit";
import { type PostingTime, slotsOn, type Slot } from "./days";
import { change, make, POST_TONES, remove, type Row, rows, run, statusWord, text } from "./data";
import { Await, Face, firstOf, Nav, nameOf, useAccounts, useAllowed, useBrands } from "./ui";

const WEEKDAYS = ["Every Day", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/** The words of a weekday as stored. */
export function weekdayWord(day: string): string {
  switch (day) {
    case "Monday":
      return t("Monday");
    case "Tuesday":
      return t("Tuesday");
    case "Wednesday":
      return t("Wednesday");
    case "Thursday":
      return t("Thursday");
    case "Friday":
      return t("Friday");
    case "Saturday":
      return t("Saturday");
    case "Sunday":
      return t("Sunday");
    default:
      return t("Every day");
  }
}

/** Whether a time is `HH:MM` (what the server takes). */
export const isTime = (v: string) => /^([01]?\d|2[0-3]):[0-5]\d$/.test(v.trim());

/** The times that serve a scope: an account's own, else (for an account) its brand's; for a brand, those with no account. */
export function timesFor(times: Row[], brand: string, account: string): { own: Row[]; shared: Row[] } {
  const ofBrand = times.filter((x) => text(x.brand) === brand && !text(x.account));
  const own = account ? times.filter((x) => text(x.account) === account) : ofBrand;
  return { own, shared: account && !own.length ? ofBrand : [] };
}

const when = (iso: string) => formatMoment(iso, { weekday: "short", day: "numeric", month: "short", hour: "numeric", minute: "2-digit" });
const DAYS_AHEAD = 14;

function Times({ brand, account, times, accountName }: { brand: string; account: string; times: Row[]; accountName: string }) {
  const client = useQueryClient();
  const canCreate = useAllowed("SocialPostingTime", "create");
  const canWrite = useAllowed("SocialPostingTime", "write");
  const canDelete = useAllowed("SocialPostingTime", "delete");
  const { own, shared } = timesFor(times, brand, account);
  const [weekday, setWeekday] = useState("Every Day");
  const [time, setTime] = useState("09:00");
  const [refused, setRefused] = useState("");
  const done = () => client.invalidateQueries({ queryKey: ["social"] });
  const add = useMutation({
    mutationFn: () => make("SocialPostingTime", { brand, account: account || null, weekday, time: time.trim(), active: true }),
    onSuccess: async () => { setRefused(""); await done(); },
    onError: (e) => setRefused(refusal(e)),
  });
  const toggle = useMutation({ mutationFn: (r: Row) => change("SocialPostingTime", r.id, { active: r.active === false }, r.version), onSuccess: done, onError: (e) => setRefused(refusal(e)) });
  const drop = useMutation({ mutationFn: (r: Row) => remove("SocialPostingTime", r.id), onSuccess: done, onError: (e) => setRefused(refusal(e)) });
  const ordered = [...own, ...shared].sort((a, b) => WEEKDAYS.indexOf(text(a.weekday) || "Every Day") - WEEKDAYS.indexOf(text(b.weekday) || "Every Day") || text(a.time).localeCompare(text(b.time)));
  return (
    <section className="card card-body stack" aria-labelledby="times-h">
      <h2 id="times-h" className="section-title">{account ? t("Times of {name}", { name: isolate(accountName) }) : t("Weekly times")}</h2>
      {account && !own.length && <p className="muted">{shared.length ? t("It has no times of its own, so it uses the brand's. Add one and it uses only its own.") : t("Neither it nor its brand has times yet.")}</p>}
      {refused && <Alert tone="error">{isolate(refused)}</Alert>}
      {ordered.length === 0 ? (
        <StatePanel kind="empty" title={t("No posting times yet")}>{t("Add the times of the week a post of this brand may go out at.")}</StatePanel>
      ) : (
        <table className="table">
          <thead><tr><th scope="col">{t("Day")}</th><th scope="col">{t("Time")}</th><th scope="col">{t("On")}</th><th scope="col"><span className="sr-only">{t("Actions")}</span></th></tr></thead>
          <tbody>
            {ordered.map((r) => {
              const mine = own.includes(r);
              return (
                <tr key={r.id}>
                  <td>{weekdayWord(text(r.weekday) || "Every Day")}</td>
                  <td className="num">{isolate(text(r.time))}</td>
                  <td>
                    <label className="row">
                      <input type="checkbox" checked={r.active !== false} disabled={!mine || !canWrite || toggle.isPending} onChange={() => toggle.mutate(r)} aria-label={t("{day} {time} is on", { day: weekdayWord(text(r.weekday) || "Every Day"), time: isolate(text(r.time)) })} />
                      {r.active === false ? <Badge>{t("Off")}</Badge> : null}
                    </label>
                  </td>
                  <td className="num">
                    {mine && canDelete && <Button size="sm" variant="danger" loading={drop.isPending && drop.variables === r} onClick={async () => { if (await confirmDialog({ title: t("Remove this time?"), message: t("Posts already in the queue keep their time."), confirmLabel: t("Remove"), danger: true })) drop.mutate(r); }}>{t("Remove")}</Button>}
                    {!mine && <span className="faint">{t("The brand's")}</span>}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      {canCreate && (
        <form className="row" onSubmit={(e) => { e.preventDefault(); if (isTime(time)) add.mutate(); }}>
          <Field label={t("Day")} htmlFor="pt-day">
            <select id="pt-day" className="select" value={weekday} onChange={(e) => setWeekday(e.target.value)}>
              {WEEKDAYS.map((d) => <option key={d} value={d}>{weekdayWord(d)}</option>)}
            </select>
          </Field>
          <Field label={t("Time")} htmlFor="pt-time" error={time && !isTime(time) ? t("A time like 09:30") : undefined}>
            <input id="pt-time" className="input" type="time" value={time} onChange={(e) => setTime(e.target.value)} />
          </Field>
          <Button type="submit" variant="primary" icon="plus" loading={add.isPending} disabled={!isTime(time)}>{t("Add the time")}</Button>
        </form>
      )}
    </section>
  );
}

export function QueueRoute() {
  const [params, setParams] = useSearchParams();
  const client = useQueryClient();
  const accounts = useAccounts();
  const brands = useBrands();
  const times = useQuery({ queryKey: ["social", "posting-times"], queryFn: () => rows("SocialPostingTime") });
  const brand = params.get("brand") ?? (brands.data?.[0]?.id ?? "");
  const account = params.get("account") ?? "";
  const put = (next: Record<string, string>) => setParams((cur) => { const o = Object.fromEntries(cur); for (const [k, v] of Object.entries(next)) v ? (o[k] = v) : delete o[k]; return o; }, { replace: true });
  const now = new Date();
  const horizon = new Date(now.getTime() + DAYS_AHEAD * 86_400_000).toISOString();
  const coming = useQuery({
    queryKey: ["social", "queue-posts", brand, account],
    queryFn: () => rows("SocialPost", [["brand", "=", brand], ["scheduled_at", "between", [now.toISOString(), horizon]], ...(account ? [["targets.account", "in", [account]] as [string, string, unknown]] : [])], { sort: "scheduled_at asc" }),
    enabled: !!brand,
  });
  const drafts = useQuery({
    queryKey: ["social", "queue-drafts", brand],
    queryFn: () => rows("SocialPost", [["brand", "=", brand], ["status", "in", ["Draft", "Rejected"]], ["scheduled_at", "is", "not set"]], { sort: "created_at desc" }).catch(() => [] as Row[]),
    enabled: !!brand,
  });
  const canMove = useAllowed("SocialPost", "write");
  const [dragged, setDragged] = useState<string | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const [refused, setRefused] = useState("");
  const [busy, setBusy] = useState<string | null>(null);

  const mine = (accounts.data ?? []).filter((a) => a.brand === brand);
  const days = Array.from({ length: DAYS_AHEAD }, (_, i) => new Date(now.getTime() + i * 86_400_000).toISOString().slice(0, 10));
  const zone = bootstrap()?.locale.timezone;
  const scope = timesFor(times.data ?? [], brand, account);
  const live = [...scope.own, ...scope.shared].map((r) => ({ brand: text(r.brand), account: text(r.account), weekday: text(r.weekday) || "Every Day", time: text(r.time), active: r.active !== false })) as PostingTime[];
  const places: Slot[] = slotsOn(live, days, zone, now, mine.map((a) => ({ id: a.id, brand: text(a.brand) }))).filter((s) => !account || !s.account || s.account === account);
  const unique = [...new Map(places.map((s) => [s.at, s])).values()].slice(0, 30);
  const posts = coming.data ?? [];
  const atPlace = (iso: string) => posts.filter((p) => Date.parse(text(p.scheduled_at)) === Date.parse(iso));
  const off = posts.filter((p) => !unique.some((s) => Date.parse(s.at) === Date.parse(text(p.scheduled_at))));

  const move = async (id: string, at: string) => {
    setBusy(id);
    setRefused("");
    try {
      await run("SocialPost", id, "reschedule", { at });
      toast(t("Moved to {at}", { at: when(at) }));
      await client.invalidateQueries({ queryKey: ["social"] });
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(null);
      setDragged(null);
      setOver(null);
    }
  };
  const queue = async (p: Row) => {
    setBusy(p.id);
    setRefused("");
    try {
      const out = await run<{ scheduled_at: string }>("SocialPost", p.id, "add_to_queue");
      toast(t("In the queue for {at}", { at: when(out.scheduled_at) }));
      await client.invalidateQueries({ queryKey: ["social"] });
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <PageStack>
      <Nav />
      <PageHead title={t("Posting times")} description={t("When a brand's posts may go out, and what is in the next places.")} actions={<ButtonLink to="/compose" variant="primary" icon="plus">{t("New post")}</ButtonLink>} />
      <Await q={firstOf(accounts, brands, times)} skeleton={<ListSkeleton rows={5} />}>
        {() =>
          !(brands.data ?? []).length ? (
            <StatePanel kind="empty" title={t("No brands yet")} action={<ButtonLink to="/e/SocialBrand/new" variant="primary">{t("Add a brand")}</ButtonLink>}>{t("Posting times belong to a brand.")}</StatePanel>
          ) : (
            <>
              <div className="row">
                <Field label={t("Brand")} htmlFor="q-brand">
                  <select id="q-brand" className="select" value={brand} onChange={(e) => put({ brand: e.target.value, account: "" })}>
                    {brands.data!.map((b) => <option key={b.id} value={b.id}>{isolate(text(b.brand_name))}</option>)}
                  </select>
                </Field>
                <Field label={t("Account")} htmlFor="q-account">
                  <select id="q-account" className="select" value={account} onChange={(e) => put({ account: e.target.value })}>
                    <option value="">{t("The brand's times")}</option>
                    {mine.map((a) => <option key={a.id} value={a.id}>{nameOf(a)}</option>)}
                  </select>
                </Field>
              </div>
              {refused && <Alert tone="error">{isolate(refused)}</Alert>}
              <Times brand={brand} account={account} times={times.data ?? []} accountName={text(mine.find((a) => a.id === account)?.account_name)} />
              <section className="card card-body stack" aria-labelledby="places-h">
                <h2 id="places-h" className="section-title">{t("The next places")}</h2>
                <Await q={coming} skeleton={<ListSkeleton rows={4} />}>
                  {() =>
                    unique.length === 0 && off.length === 0 ? (
                      <StatePanel kind="empty" title={t("No place in the next {n} days", { n: String(DAYS_AHEAD) })}>{t("Add a posting time above and its places show here.")}</StatePanel>
                    ) : (
                      <ul className="stack" aria-label={t("The next places")}>
                        {unique.map((s) => {
                          const there = atPlace(s.at);
                          return (
                            <li key={s.at} className="row cal-place-row" data-over={over === s.at || undefined} data-at={s.at}
                              onDragOver={(e) => { if (dragged) { e.preventDefault(); setOver(s.at); } }} onDragLeave={() => setOver((o) => (o === s.at ? null : o))}
                              onDrop={(e) => { e.preventDefault(); if (dragged) void move(dragged, s.at); }}>
                              <span className="cal-chip-time">{when(s.at)}</span>
                              {there.length === 0 ? (
                                <>
                                  <span className="faint spacer">{t("Free")}</span>
                                  <ButtonLink size="sm" to={`/compose?at=${encodeURIComponent(s.at)}`}>{t("Write a post")}</ButtonLink>
                                </>
                              ) : (
                                there.map((p) => <PlacePost key={p.id} p={p} places={unique.filter((x) => !atPlace(x.at).length)} canMove={canMove} busy={busy === p.id} onDrag={setDragged} onMove={move} />)
                              )}
                            </li>
                          );
                        })}
                        {off.map((p) => (
                          <li key={p.id} className="row">
                            <span className="cal-chip-time">{when(text(p.scheduled_at))}</span>
                            <PlacePost p={p} places={unique.filter((x) => !atPlace(x.at).length)} canMove={canMove} busy={busy === p.id} onDrag={setDragged} onMove={move} />
                            <Badge>{t("Outside the posting times")}</Badge>
                          </li>
                        ))}
                      </ul>
                    )
                  }
                </Await>
              </section>
              <section className="card card-body stack" aria-labelledby="drafts-h">
                <h2 id="drafts-h" className="section-title">{t("Drafts to queue")}</h2>
                <Await q={drafts} skeleton={<ListSkeleton rows={3} />}>
                  {(list) =>
                    list.length === 0 ? (
                      <p className="muted">{t("No draft is waiting for a time.")}</p>
                    ) : (
                      <ul className="stack">
                        {list.map((p) => (
                          <li key={p.id} className="row">
                            <Link className="spacer" to={`/compose/${p.id}`}>{isolate(text(p.headline))}</Link>
                            <StatusBadge value={text(p.status)} tones={POST_TONES} label={statusWord(text(p.status))} />
                            {canMove && <Button size="sm" loading={busy === p.id} onClick={() => void queue(p)}>{t("Add to queue")}</Button>}
                          </li>
                        ))}
                      </ul>
                    )
                  }
                </Await>
              </section>
            </>
          )
        }
      </Await>
    </PageStack>
  );
}

function PlacePost({ p, places, canMove, busy, onDrag, onMove }: { p: Row; places: Slot[]; canMove: boolean; busy: boolean; onDrag: (id: string | null) => void; onMove: (id: string, at: string) => Promise<void> }) {
  const status = text(p.status);
  const movable = canMove && ["Draft", "Rejected", "In Review", "Approved"].includes(status);
  return (
    <span className="row spacer" draggable={movable} onDragStart={() => onDrag(p.id)} onDragEnd={() => onDrag(null)}>
      <Face name={text(p.headline)} />
      <Link to={`/compose/${p.id}`}>{isolate(text(p.headline))}</Link>
      <StatusBadge value={status} tones={POST_TONES} label={statusWord(status)} />
      {movable && places.length > 0 && (
        <select className="select" aria-label={t("Move {name} to a free place", { name: isolate(text(p.headline)) })} disabled={busy} value="" onChange={(e) => e.target.value && void onMove(p.id, e.target.value)}>
          <option value="">{t("Move to…")}</option>
          {places.map((s) => <option key={s.at} value={s.at}>{when(s.at)}</option>)}
        </select>
      )}
    </span>
  );
}
