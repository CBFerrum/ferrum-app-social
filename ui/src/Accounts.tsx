// Accounts (/accounts): a brand's accounts with how each is reached, connecting and reconnecting, checking, signing out,
// choosing which of the pages or organisations a sign-in reaches (each becomes an account), adding an account by a
// connection, and their order. Calls: SocialAccount list, actions connect / check / disconnect / reachable / choose,
// PATCH sort_order, POST SocialAccount; the server's words are shown as they come.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router";
import { api, errorMessage, isolate, refusal, t } from "@ferrum/client";
import { Alert, Badge, Button, ButtonLink, confirmDialog, Dialog, Field, ListSkeleton, PageHead, PageStack, StatePanel, StatusBadge, toast } from "@ferrum/kit";
import { change, make, type Row, rows, run, STATE_TONES, stateWord, text } from "./data";
import { Await, brandName, byOrder, Face, firstOf, Nav, nameOf, useAccounts, useAllowed, useBrands } from "./ui";

/** What `reachable` lists: `{id, name, handle, picture, kind, account}`; `account` is the Social account that has it. */
export interface Reached {
  id: string;
  name: string | null;
  handle: string | null;
  picture: string | null;
  kind: string | null;
  account: string | null;
}

/** The answer of `connect`: a sign-in that leaves the page (POST `connect.path`), or a session that signed in at once. */
export interface Connected {
  connect?: { method: string; path: string };
  signed_in?: unknown;
  state?: string;
  note?: string | null;
}

/** Where a sign-in that leaves the page goes: the address the platform's answer names (`url`, `authorize_url` or `redirect`), else nowhere. */
export function leaveFor(answer: unknown): string | null {
  const a = (answer ?? {}) as Record<string, unknown>;
  for (const k of ["url", "authorize_url", "redirect", "location"]) if (typeof a[k] === "string" && a[k]) return a[k] as string;
  return null;
}

/** Swaps the order of two neighbours: the order values to write (each account's `sort_order`, 0, 1, 2 … by the list shown). */
export function reorder(list: Row[], id: string, by: -1 | 1): [string, number][] {
  const i = list.findIndex((a) => a.id === id);
  const j = i + by;
  if (i < 0 || j < 0 || j >= list.length) return [];
  const next = [...list];
  [next[i], next[j]] = [next[j], next[i]];
  return next.map((a, n) => [a.id, n] as [string, number]).filter(([aid, n]) => Number(list.find((a) => a.id === aid)?.sort_order ?? 0) !== n);
}

function Picker({ account, onClose }: { account: Row; onClose: () => void }) {
  const client = useQueryClient();
  const q = useQuery({ queryKey: ["social", "reachable", account.id], queryFn: () => run<{ accounts: Reached[] }>("SocialAccount", account.id, "reachable").then((r) => r.accounts), retry: false });
  const [picked, setPicked] = useState<string[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState("");
  const chosen = picked ?? (q.data ?? []).filter((r) => r.account).map((r) => r.id);
  const toggle = (id: string) => setPicked(chosen.includes(id) ? chosen.filter((x) => x !== id) : [...chosen, id]);
  const save = async () => {
    setBusy(true);
    setRefused("");
    try {
      const out = await run<{ accounts: { created: boolean }[] }>("SocialAccount", account.id, "choose", { picked: chosen });
      toast(t("{n} accounts chosen, {made} new", { n: String(out.accounts.length), made: String(out.accounts.filter((a) => a.created).length) }));
      await client.invalidateQueries({ queryKey: ["social"] });
      onClose();
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      open
      onClose={onClose}
      title={t("Choose accounts")}
      description={t("What the sign-in of {name} reaches. Each one you choose is an account of its own.", { name: isolate(text(account.account_name)) })}
      footer={
        <>
          <Button onClick={onClose}>{t("Cancel")}</Button>
          <Button variant="primary" loading={busy} disabled={!chosen.length} onClick={() => void save()}>
            {t("Use the chosen")}
          </Button>
        </>
      }
    >
      {refused && <Alert tone="error">{refused}</Alert>}
      <Await q={q} skeleton={<ListSkeleton rows={4} />}>
        {(list) =>
          list.length ? (
            <ul className="stack" role="group" aria-label={t("What the sign-in reaches")}>
              {list.map((r) => (
                <li key={r.id} className="row">
                  <label className="row">
                    <input type="checkbox" checked={chosen.includes(r.id)} onChange={() => toggle(r.id)} />
                    <Face name={r.name ?? r.id} picture={r.picture} />
                    <span>
                      {isolate(r.name ?? r.id)}
                      {r.handle && <span className="muted"> {isolate(`@${r.handle}`)}</span>}
                      {r.kind && <span className="muted"> · {isolate(r.kind)}</span>}
                    </span>
                  </label>
                  {r.account && <Badge tone="green">{t("Already an account")}</Badge>}
                </li>
              ))}
            </ul>
          ) : (
            <StatePanel kind="empty" title={t("The sign-in reaches nothing to choose")} />
          )
        }
      </Await>
    </Dialog>
  );
}

function AddAccount({ brands, onClose }: { brands: Row[]; onClose: () => void }) {
  const client = useQueryClient();
  const profiles = useQuery({ queryKey: ["social", "profiles"], queryFn: () => rows("SocialProfile", [], { sort: "profile_name asc" }) });
  // Connections are an administrator's to list; anyone else types the name (the server takes its name or its id).
  const known = useQuery({ queryKey: ["social", "connections"], queryFn: () => api.get<Row[] | { connections: Row[] }>("/api/admin/connections").then((r) => (Array.isArray(r) ? r : r.connections ?? [])), retry: false });
  const [v, setV] = useState({ account_name: "", brand: "", handle: "", connection: "", profile: "" });
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState("");
  const set = (k: keyof typeof v) => (e: { target: { value: string } }) => setV({ ...v, [k]: e.target.value });
  const save = async () => {
    setBusy(true);
    setRefused("");
    try {
      await make("SocialAccount", { ...v, profile: v.profile || null });
      await client.invalidateQueries({ queryKey: ["social"] });
      toast(t("Account added. Connect it next."));
      onClose();
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      open
      onClose={onClose}
      title={t("Add an account")}
      footer={
        <>
          <Button onClick={onClose}>{t("Cancel")}</Button>
          <Button variant="primary" loading={busy} disabled={!v.account_name || !v.brand || !v.connection || !v.handle} onClick={() => void save()}>
            {t("Add")}
          </Button>
        </>
      }
    >
      {refused && <Alert tone="error">{refused}</Alert>}
      <div className="stack">
        <Field label={t("Name")} required htmlFor="acc-name">
          <input id="acc-name" className="input" value={v.account_name} onChange={set("account_name")} />
        </Field>
        <Field label={t("Brand")} required htmlFor="acc-brand">
          <select id="acc-brand" className="select" value={v.brand} onChange={set("brand")}>
            <option value="">{t("Choose a brand")}</option>
            {brands.map((b) => (
              <option key={b.id} value={b.id}>
                {isolate(text(b.brand_name))}
              </option>
            ))}
          </select>
        </Field>
        <Field label={t("Connection")} required htmlFor="acc-conn" hint={t("The connection that reaches it, by its name or id.")}>
          <input id="acc-conn" className="input" list="acc-conns" value={v.connection} onChange={set("connection")} />
          <datalist id="acc-conns">
            {(known.data ?? []).map((c) => (
              <option key={text(c.id)} value={text(c.name)} />
            ))}
          </datalist>
        </Field>
        <Field label={t("Its name on the network")} required htmlFor="acc-handle">
          <input id="acc-handle" className="input" value={v.handle} onChange={set("handle")} />
        </Field>
        <Field label={t("What its network takes")} htmlFor="acc-profile" hint={t("Needed only when the connection does not say.")}>
          <select id="acc-profile" className="select" value={v.profile} onChange={set("profile")}>
            <option value="">{t("The connection says")}</option>
            {(profiles.data ?? []).map((p) => (
              <option key={p.id} value={p.id}>
                {isolate(text(p.profile_name))}
              </option>
            ))}
          </select>
        </Field>
      </div>
    </Dialog>
  );
}

function AccountCard({ a, first, last, canWrite, onMove, onChoose }: { a: Row; first: boolean; last: boolean; canWrite: boolean; onMove: (by: -1 | 1) => void; onChoose: () => void }) {
  const client = useQueryClient();
  const [busy, setBusy] = useState<string | null>(null);
  const [refused, setRefused] = useState("");
  const after = () => client.invalidateQueries({ queryKey: ["social", "accounts"] });
  const doing = async (name: "connect" | "check" | "disconnect") => {
    if (name === "disconnect" && !(await confirmDialog({ title: t("Sign this account out?"), message: t("The provider is told to revoke its access."), confirmLabel: t("Disconnect"), danger: true }))) return;
    setBusy(name);
    setRefused("");
    try {
      const out = await run<Connected & { read?: unknown }>("SocialAccount", a.id, name);
      if (out.connect) {
        // A sign-in that leaves the page and comes back: the platform's broker starts it.
        const started = await api.post<unknown>(out.connect.path, {});
        const to = leaveFor(started);
        if (to) {
          window.location.assign(to);
          return;
        }
      } else if (name === "connect") toast(out.note || t("Signed in: {state}", { state: text(out.state) }));
      else if (name === "disconnect") toast(t("Signed out"));
      else toast(out.note || t("Checked: {state}", { state: text(out.state) }));
      await after();
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(null);
    }
  };
  const state = text(a.state);
  return (
    <li className="card card-body stack" aria-label={text(a.account_name)}>
      <div className="row">
        <Face name={text(a.account_name)} picture={text(a.picture)} />
        <div className="spacer">
          <strong>{nameOf(a)}</strong>
          <div className="muted">{isolate([text(a.network), text(a.handle) && `@${text(a.handle)}`].filter(Boolean).join(" · "))}</div>
        </div>
        <StatusBadge value={state} tones={STATE_TONES} label={stateWord(state)} />
        {a.active === false && <Badge>{t("Paused")}</Badge>}
      </div>
      {text(a.state_note) && <p className="muted">{isolate(text(a.state_note))}</p>}
      {refused && <Alert tone="error">{refused}</Alert>}
      <div className="row">
        {canWrite && (
          <>
            <Button size="sm" variant={state === "Connected" ? "default" : "primary"} loading={busy === "connect"} onClick={() => void doing("connect")}>
              {state === "Connected" ? t("Reconnect") : t("Connect")}
            </Button>
            <Button size="sm" loading={busy === "check"} onClick={() => void doing("check")}>
              {t("Check")}
            </Button>
            {state === "Connected" && (
              <Button size="sm" variant={a.network_id ? "default" : "primary"} onClick={onChoose}>
                {t("Choose accounts")}
              </Button>
            )}
            <Button size="sm" variant="danger" loading={busy === "disconnect"} onClick={() => void doing("disconnect")}>
              {t("Disconnect")}
            </Button>
          </>
        )}
        <ButtonLink size="sm" to={`/e/SocialAccount/${a.id}`}>
          {t("Preferences")}
        </ButtonLink>
        <span className="spacer" />
        {canWrite && (
          <>
            <Button size="sm" icon="up" aria-label={t("Move {name} up", { name: isolate(text(a.account_name)) })} disabled={first} onClick={() => onMove(-1)} />
            <Button size="sm" icon="down" aria-label={t("Move {name} down", { name: isolate(text(a.account_name)) })} disabled={last} onClick={() => onMove(1)} />
          </>
        )}
      </div>
    </li>
  );
}

export function AccountsRoute() {
  const client = useQueryClient();
  const accounts = useAccounts();
  const brands = useBrands();
  const canWrite = useAllowed("SocialAccount", "write");
  const canCreate = useAllowed("SocialAccount", "create");
  const [picking, setPicking] = useState<Row | null>(null);
  const [adding, setAdding] = useState(false);
  const move = useMutation({
    mutationFn: async ({ list, id, by }: { list: Row[]; id: string; by: -1 | 1 }) => {
      for (const [aid, n] of reorder(list, id, by)) await change("SocialAccount", aid, { sort_order: n });
    },
    onSuccess: () => client.invalidateQueries({ queryKey: ["social", "accounts"] }),
    onError: (e) => toast(errorMessage(e), "error"),
  });
  return (
    <PageStack>
      <Nav />
      <PageHead
        title={t("Accounts")}
        description={t("Each brand's accounts, and whether they can be reached.")}
        actions={
          canCreate && (
            <Button variant="primary" icon="plus" onClick={() => setAdding(true)}>
              {t("Add an account")}
            </Button>
          )
        }
      />
      <Await q={firstOf(accounts, brands)} skeleton={<ListSkeleton rows={4} />}>
        {() => {
          const list = [...(accounts.data ?? [])].sort(byOrder);
          const bs = brands.data ?? [];
          if (!list.length)
            return (
              <StatePanel kind="empty" title={t("No accounts yet")} action={canCreate ? <Button variant="primary" onClick={() => setAdding(true)}>{t("Add an account")}</Button> : undefined}>
                {t("Add an account of a brand, then connect it.")}
              </StatePanel>
            );
          const groups = [...new Set(list.map((a) => text(a.brand)))];
          return (
            <>
              {groups.map((brand) => {
                const own = list.filter((a) => text(a.brand) === brand);
                return (
                  <section key={brand} aria-labelledby={`brand-${brand}`} className="stack">
                    <h2 id={`brand-${brand}`} className="section-title">
                      {isolate(brandName(bs, brand) || t("No brand"))}
                    </h2>
                    <ul className="stack">
                      {own.map((a, i) => (
                        <AccountCard key={a.id} a={a} first={i === 0} last={i === own.length - 1} canWrite={canWrite} onMove={(by) => move.mutate({ list: own, id: a.id, by })} onChoose={() => setPicking(a)} />
                      ))}
                    </ul>
                  </section>
                );
              })}
              <p className="muted">
                <Link to="/e/SocialBrand">{t("Brands")}</Link>
              </p>
            </>
          );
        }}
      </Await>
      {picking && <Picker account={picking} onClose={() => setPicking(null)} />}
      {adding && <AddAccount brands={brands.data ?? []} onClose={() => setAdding(false)} />}
    </PageStack>
  );
}
