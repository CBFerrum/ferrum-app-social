// The inbox (/inbox) as a desk: conversations on the left (filters: account, brand, kind, status, assigned to me), the thread in
// the middle with the post it answers, a reply box with saved replies and mentions, assign and close, and the contact the sender
// was matched to. Keys: j / k between conversations, r to reply, e to close. Calls: SocialMessage list/PATCH, action reply,
// SocialReply list, SocialPublication list (the post a comment answers), SocialSavedReply list, SocialAccount `mentions`.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router";
import { api, docPath, formatMoment, isolate, refusal, t } from "@ferrum/client";
import { Alert, Badge, Button, EmptyState, Field, ListSkeleton, PageHead, PageStack, StatusBadge, toast, useDebounced, type Tone } from "@ferrum/kit";
import { useMe } from "@ferrum/records";
import { type Filter, change, record, type Row, rows, run, text } from "./data";
import { mentionToken } from "./composer/Items";
import { Await, brandName, Face, firstOf, Nav, nameOf, useAccounts, useAllowed, useBrands } from "./ui";

const STATUSES = ["New", "Assigned", "Answered", "Closed"];
const KINDS = ["Comment", "Mention", "Direct"];
const TONES: Record<string, Tone> = { New: "red", Assigned: "amber", Answered: "green", Closed: "neutral" };

export function statusWord(s: string): string {
  switch (s) {
    case "New":
      return t("New");
    case "Assigned":
      return t("Assigned");
    case "Answered":
      return t("Answered");
    default:
      return t("Closed");
  }
}
export function kindWord(k: string): string {
  switch (k) {
    case "Mention":
      return t("Mention");
    case "Direct":
      return t("Direct message");
    default:
      return t("Comment");
  }
}

export interface InboxFilters {
  account: string;
  brand: string;
  kind: string;
  status: string;
  mine: boolean;
}

/** The server filters for the conversations shown. */
export function filtersFor(f: InboxFilters, me: string): Filter[] {
  const out: Filter[] = [];
  if (f.account) out.push(["account", "=", f.account]);
  if (f.brand) out.push(["account.brand", "=", f.brand]);
  if (f.kind) out.push(["kind", "=", f.kind]);
  out.push(f.status ? ["status", "=", f.status] : ["status", "in", ["New", "Assigned", "Answered"]]);
  if (f.mine && me) out.push(["assigned_to", "=", me]);
  return out;
}

function Contact({ message }: { message: Row }) {
  const entity = text(message.contact_entity);
  const id = text(message.contact_id);
  const q = useQuery({ queryKey: ["social", "contact", entity, id], queryFn: () => api.get<Row>(docPath(entity, id)), enabled: !!entity && !!id, retry: false });
  if (!entity || !id) return <p className="muted">{t("The sender is not matched to a contact.")}</p>;
  if (q.isError) return <p className="muted">{isolate(refusal(q.error))}</p>;
  const shown = Object.entries(q.data ?? {}).filter(([k, v]) => typeof v === "string" && v && !["id", "version", "created_at", "updated_at", "tenant_id"].includes(k) && !/^[0-9a-f-]{36}$/.test(String(v))).slice(0, 6);
  return (
    <section className="card card-body stack" aria-label={t("The contact")}>
      <h3 className="section-title">{t("The contact")}</h3>
      <dl className="kv">
        {shown.map(([k, v]) => (
          <div key={k}><dt>{isolate(k.replace(/_/g, " "))}</dt><dd>{isolate(String(v))}</dd></div>
        ))}
      </dl>
    </section>
  );
}

function Thread({ message, onChanged }: { message: Row; onChanged: () => void }) {
  const client = useQueryClient();
  const me = (useMe() as { data?: { user: { email: string } } }).data?.user.email ?? "";
  const canWrite = useAllowed("SocialMessage", "write");
  const thread = text(message.thread);
  const same = useQuery({ queryKey: ["social", "thread", message.id, thread], queryFn: () => (thread ? rows("SocialMessage", [["thread", "=", thread]], { sort: "created_at asc" }) : Promise.resolve([message])) });
  const ids = (same.data ?? [message]).map((m) => m.id);
  const replies = useQuery({ queryKey: ["social", "replies", ids.join(",")], queryFn: () => rows("SocialReply", [["message", "in", ids]], { sort: "created_at asc" }).catch(() => [] as Row[]) });
  const answered = useQuery({
    queryKey: ["social", "answers", message.post_external_id],
    queryFn: async () => {
      const pubs = await rows("SocialPublication", [["external_id", "=", text(message.post_external_id)]]);
      const post = text(pubs[0]?.post);
      return post ? record("SocialPost", post) : null;
    },
    enabled: !!text(message.post_external_id),
    retry: false,
  });
  const saved = useQuery({ queryKey: ["social", "saved-replies"], queryFn: () => rows("SocialSavedReply", [], { sort: "title asc" }).catch(() => [] as Row[]) });
  const [reply, setReply] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [refused, setRefused] = useState("");
  const [assignee, setAssignee] = useState("");
  const area = useRef<HTMLTextAreaElement>(null);
  useEffect(() => { setReply(""); setRefused(""); setAssignee(text(message.assigned_to)); }, [message.id]);
  // Keys: r puts the cursor in the reply box.
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (e.key === "r" && !/^(INPUT|TEXTAREA|SELECT)$/.test((e.target as HTMLElement).tagName) && !e.metaKey && !e.ctrlKey) {
        e.preventDefault();
        area.current?.focus();
      }
    };
    window.addEventListener("keydown", on);
    return () => window.removeEventListener("keydown", on);
  }, []);
  const asking = mentionToken(reply, reply.length);
  const query = useDebounced(asking?.query ?? "", 250);
  const found = useQuery({
    queryKey: ["social", "mentions", message.account, query],
    queryFn: () => run<{ mentions: { value: string; label: string; text: string }[] }>("SocialAccount", text(message.account), "mentions", { query }).then((r) => r.mentions),
    enabled: !!text(message.account) && !!asking && query.length > 0,
    retry: false,
  });
  const done = async () => {
    await client.invalidateQueries({ queryKey: ["social"] });
    onChanged();
  };
  const send = async (params: Record<string, unknown>) => {
    setBusy("reply");
    setRefused("");
    try {
      await run("SocialMessage", message.id, "reply", params);
      setReply("");
      toast(t("Reply sent to the network's queue"));
      await done();
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(null);
    }
  };
  const patch = async (what: string, values: Record<string, unknown>) => {
    setBusy(what);
    setRefused("");
    try {
      await change("SocialMessage", message.id, values, message.version);
      await done();
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(null);
    }
  };
  const all = same.data ?? [message];
  const ours = replies.data ?? [];
  const entries = [...all.map((m) => ({ at: text(m.created_at), who: text(m.author), words: text(m.text), ours: false, id: m.id, failure: "" })), ...ours.map((r) => ({ at: text(r.created_at), who: text(r.author) || t("You"), words: text(r.text), ours: true, id: r.id, failure: text(r.failure) }))].sort((a, b) => a.at.localeCompare(b.at));
  const closed = text(message.status) === "Closed";
  return (
    <div className="stack">
      <div className="row">
        <h2 className="section-title spacer">{isolate(text(message.author))} · {kindWord(text(message.kind))}</h2>
        <StatusBadge value={text(message.status)} tones={TONES} label={statusWord(text(message.status))} />
      </div>
      {answered.data && (
        <p className="muted">{t("It answers")} <Link to={`/compose/${answered.data.id}`}>{isolate(text(answered.data.headline))}</Link></p>
      )}
      <ul className="stack" aria-label={t("The conversation")}>
        {entries.map((e) => (
          <li key={e.id} className={e.ours ? "card card-body inbox-ours" : "card card-body"}>
            <div className="muted">{isolate(e.who)} · {e.at ? formatMoment(e.at, { dateStyle: "medium", timeStyle: "short" }) : ""}</div>
            <p className="preview-text">{isolate(e.words)}</p>
            {e.failure && <Alert tone="error">{isolate(e.failure)}</Alert>}
          </li>
        ))}
      </ul>
      {refused && <Alert tone="error">{isolate(refused)}</Alert>}
      {canWrite && !closed && (
        <div className="stack">
          <div className="row">
            <select className="select" aria-label={t("A saved reply")} value="" onChange={(e) => { const s = saved.data?.find((x) => x.id === e.target.value); if (s) setReply(text(s.text)); }}>
              <option value="">{t("A saved reply…")}</option>
              {(saved.data ?? []).map((s) => <option key={s.id} value={s.id}>{isolate(text(s.title))}</option>)}
            </select>
          </div>
          <Field label={t("Your reply")} htmlFor="inbox-reply">
            <textarea id="inbox-reply" ref={area} className="textarea" rows={3} value={reply} onChange={(e) => setReply(e.target.value)} />
          </Field>
          {asking && found.data && found.data.length > 0 && (
            <ul className="listbox" role="listbox" aria-label={t("People to mention")}>
              {found.data.map((p) => (
                <li key={p.value} role="option" aria-selected={false}><button type="button" className="option" onClick={() => setReply(reply.slice(0, asking.start) + p.text + " ")}>{isolate(p.label)} <span className="muted">{isolate(p.value)}</span></button></li>
              ))}
            </ul>
          )}
          <div className="row">
            <Button variant="primary" loading={busy === "reply"} disabled={!reply.trim()} onClick={() => void send({ reply })}>{t("Send the reply")}</Button>
          </div>
        </div>
      )}
      {canWrite && (
        <div className="row">
          <Button size="sm" loading={busy === "me"} disabled={!me} onClick={() => void patch("me", { assigned_to: me, status: "Assigned" })}>{t("Assign to me")}</Button>
          <Field label={t("Assign to")} htmlFor="inbox-assign"><input id="inbox-assign" className="input" type="email" value={assignee} onChange={(e) => setAssignee(e.target.value)} /></Field>
          <Button size="sm" loading={busy === "assign"} disabled={!assignee.trim()} onClick={() => void patch("assign", { assigned_to: assignee.trim(), status: "Assigned" })}>{t("Assign")}</Button>
          <span className="spacer" />
          {closed ? <Button size="sm" loading={busy === "open"} onClick={() => void patch("open", { status: "New" })}>{t("Reopen")}</Button> : <Button size="sm" loading={busy === "close"} onClick={() => void patch("close", { status: "Closed" })}>{t("Close (e)")}</Button>}
        </div>
      )}
      <Contact message={message} />
    </div>
  );
}

export function InboxRoute() {
  const [params, setParams] = useSearchParams();
  const client = useQueryClient();
  const accounts = useAccounts();
  const brands = useBrands();
  const me = (useMe() as { data?: { user: { email: string } } }).data?.user.email ?? "";
  const f: InboxFilters = { account: params.get("account") ?? "", brand: params.get("brand") ?? "", kind: params.get("kind") ?? "", status: params.get("status") ?? "", mine: params.get("mine") === "1" };
  const put = (next: Record<string, string>) => setParams((cur) => { const o = Object.fromEntries(cur); for (const [k, v] of Object.entries(next)) v ? (o[k] = v) : delete o[k]; return o; }, { replace: true });
  const list = useQuery({ queryKey: ["social", "inbox-list", f, me], queryFn: () => rows("SocialMessage", filtersFor(f, me), { sort: "updated_at desc" }) });
  const open = params.get("m") ?? "";
  const messages = list.data ?? [];
  const current = messages.find((m) => m.id === open) ?? messages[0];
  const index = useMemo(() => messages.findIndex((m) => m.id === current?.id), [messages, current]);
  const closeKey = useRef<() => void>(() => undefined);
  closeKey.current = () => {
    if (!current || text(current.status) === "Closed") return;
    void change("SocialMessage", current.id, { status: "Closed" }, current.version).then(() => client.invalidateQueries({ queryKey: ["social"] })).catch((e) => toast(refusal(e), "error"));
  };
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (/^(INPUT|TEXTAREA|SELECT)$/.test((e.target as HTMLElement).tagName) || e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key === "j" || e.key === "k") {
        const next = messages[index + (e.key === "j" ? 1 : -1)];
        if (next) put({ m: next.id });
      } else if (e.key === "e") closeKey.current();
    };
    window.addEventListener("keydown", on);
    return () => window.removeEventListener("keydown", on);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [messages, index]);
  const accountOf = (m: Row) => accounts.data?.find((a) => a.id === m.account);
  return (
    <PageStack>
      <Nav />
      <PageHead title={t("Inbox")} description={t("Comments, mentions and direct messages to your accounts. j and k move, r replies, e closes.")} />
      <div className="row">
        <Field label={t("Brand")} htmlFor="in-brand"><select id="in-brand" className="select" value={f.brand} onChange={(e) => put({ brand: e.target.value, account: "" })}><option value="">{t("All brands")}</option>{(brands.data ?? []).map((b) => <option key={b.id} value={b.id}>{isolate(text(b.brand_name))}</option>)}</select></Field>
        <Field label={t("Account")} htmlFor="in-account"><select id="in-account" className="select" value={f.account} onChange={(e) => put({ account: e.target.value })}><option value="">{t("All accounts")}</option>{(accounts.data ?? []).filter((a) => !f.brand || a.brand === f.brand).map((a) => <option key={a.id} value={a.id}>{nameOf(a)}</option>)}</select></Field>
        <Field label={t("Kind")} htmlFor="in-kind"><select id="in-kind" className="select" value={f.kind} onChange={(e) => put({ kind: e.target.value })}><option value="">{t("Any kind")}</option>{KINDS.map((k) => <option key={k} value={k}>{kindWord(k)}</option>)}</select></Field>
        <Field label={t("Status")} htmlFor="in-status"><select id="in-status" className="select" value={f.status} onChange={(e) => put({ status: e.target.value })}><option value="">{t("Open ones")}</option>{STATUSES.map((s) => <option key={s} value={s}>{statusWord(s)}</option>)}</select></Field>
        <label className="row"><input type="checkbox" checked={f.mine} onChange={(e) => put({ mine: e.target.checked ? "1" : "" })} />{t("Assigned to me")}</label>
      </div>
      <Await q={firstOf(accounts, brands, list)} skeleton={<ListSkeleton rows={6} />}>
        {() =>
          messages.length === 0 ? (
            <EmptyState title={t("Nothing to answer")}>{t("Every message is answered.")}</EmptyState>
          ) : (
            <div className="inbox-desk">
              <ul className="inbox-list" aria-label={t("Conversations")}>
                {messages.map((m) => {
                  const a = accountOf(m);
                  return (
                    <li key={m.id}>
                      <button type="button" className="inbox-row" aria-current={m.id === current?.id || undefined} onClick={() => put({ m: m.id })}>
                        <Face name={text(m.author)} picture={text(a?.picture)} small />
                        <span className="inbox-text">
                          <strong>{isolate(text(m.author))}</strong>
                          <span className="muted inbox-snippet">{isolate(text(m.text).slice(0, 100))}</span>
                          <span className="muted">{a ? nameOf(a) : ""} · {brandName(brands.data, a?.brand)}</span>
                        </span>
                        <Badge tone={TONES[text(m.status)] ?? "neutral"}>{statusWord(text(m.status))}</Badge>
                      </button>
                    </li>
                  );
                })}
              </ul>
              <div className="inbox-thread">{current && <Thread message={current} onChanged={() => undefined} />}</div>
            </div>
          )
        }
      </Await>
    </PageStack>
  );
}
