// Numbers (/numbers): a period and the one before it, the headline figures against that period, followers by day, numbers by
// network, by account and by kind of post, the top posts by a number chosen, and the best times. Each is one of the app's reports
// (`GET /api/reports/{name}`). The kit has no chart pieces, so the figures are kit Stat cards and tables, with a small line of
// plain SVG for a series. A report can be downloaded as CSV; the brand's week as a document.

import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api, formatDecimal, formatMoment, isolate, t, todayIso } from "@ferrum/client";
import { Button, EmptyState, Field, ListSkeleton, PageHead, PageStack, Stat, StatGrid } from "@ferrum/kit";
import { type Row, text } from "./data";
import { before, changeText, lastDays, toCsv } from "./period";
import { Line } from "./PostNumbers";
import { Await, Nav, useBrands } from "./ui";

interface Report {
  columns: { name: string; label?: string; type?: string }[];
  rows: Record<string, unknown>[];
  meta?: Record<string, string> | null;
}

const useReport = (name: string, params: Record<string, string>) =>
  useQuery({ queryKey: ["social", "report", name, params], queryFn: () => api.get<Report>(`/api/reports/${name}`, params), retry: false });

/** A cell as its column says: whole numbers grouped, dates in the person's form, text isolated. */
function cell(v: unknown, type?: string) {
  if (v == null || v === "") return "–";
  if (type === "int") return formatDecimal(Number(v), 0);
  if (type === "decimal") return formatDecimal(Number(v), 1);
  if (type === "date" || type === "datetime") return formatMoment(String(v), type === "date" ? { dateStyle: "medium" } : { dateStyle: "medium", timeStyle: "short" });
  return isolate(String(v));
}

function download(name: string, r: Report) {
  const url = URL.createObjectURL(new Blob(["﻿" + toCsv(r.columns, r.rows)], { type: "text/csv;charset=utf-8" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = `${name}-${todayIso()}.csv`;
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 30_000);
}

function Table({ name, title, params, extra }: { name: string; title: string; params: Record<string, string>; extra?: React.ReactNode }) {
  const q = useReport(name, params);
  return (
    <section className="card card-body stack" aria-label={title}>
      <div className="row">
        <h2 className="section-title spacer">{title}</h2>
        {extra}
        {q.data && q.data.rows.length > 0 && <Button size="sm" icon="download" onClick={() => download(name, q.data!)}>{t("Download CSV")}</Button>}
      </div>
      <Await q={q} skeleton={<ListSkeleton rows={3} />}>
        {(r) =>
          r.rows.length === 0 ? (
            <p className="muted">{t("Nothing in this period.")}</p>
          ) : (
            <div className="table-wrap" tabIndex={0} role="region" aria-label={title}>
              <table className="table">
                <thead><tr>{r.columns.map((c) => <th key={c.name} scope="col" className={c.type === "int" || c.type === "decimal" ? "num" : undefined}>{isolate(c.label ?? c.name)}</th>)}</tr></thead>
                <tbody>{r.rows.map((row, i) => <tr key={i}>{r.columns.map((c) => <td key={c.name} className={c.type === "int" || c.type === "decimal" ? "num" : undefined}>{cell(row[c.name], c.type)}</td>)}</tr>)}</tbody>
              </table>
            </div>
          )
        }
      </Await>
    </section>
  );
}

function Followers({ params }: { params: Record<string, string> }) {
  const q = useReport("social_followers_by_day", params);
  return (
    <section className="card card-body stack" aria-label={t("Followers by day")}>
      <div className="row">
        <h2 className="section-title spacer">{t("Followers by day")}</h2>
        {q.data && q.data.rows.length > 0 && <Button size="sm" icon="download" onClick={() => download("social_followers_by_day", q.data!)}>{t("Download CSV")}</Button>}
      </div>
      <Await q={q} skeleton={<ListSkeleton rows={3} />}>
        {(r) => {
          const by = new Map<string, number[]>();
          for (const row of r.rows) by.set(String(row.account), [...(by.get(String(row.account)) ?? []), Number(row.followers)]);
          return by.size === 0 ? (
            <p className="muted">{t("No account has told its followers in this period.")}</p>
          ) : (
            <table className="table">
              <thead><tr><th scope="col">{t("Account")}</th><th scope="col" className="num">{t("Followers")}</th><th scope="col" className="num">{t("Change")}</th><th scope="col"><span className="sr-only">{t("Day by day")}</span></th></tr></thead>
              <tbody>
                {[...by.entries()].map(([account, v]) => (
                  <tr key={account}>
                    <td>{isolate(account)}</td>
                    <td className="num">{formatDecimal(v[v.length - 1], 0)}</td>
                    <td className="num">{formatDecimal(v[v.length - 1] - v[0], 0)}</td>
                    <td><Line values={v} label={t("Followers of {name} by day", { name: isolate(account) })} /></td>
                  </tr>
                ))}
              </tbody>
            </table>
          );
        }}
      </Await>
    </section>
  );
}

export function NumbersRoute() {
  const brands = useBrands();
  const [days, setDays] = useState(28);
  const [custom, setCustom] = useState<{ from: string; to: string } | null>(null);
  const [brand, setBrand] = useState("");
  const [by, setBy] = useState("engagements");
  const span = custom ?? lastDays(days);
  const params: Record<string, string> = { from_date: span.from, to_date: span.to, ...(brand ? { brand } : {}) };
  const compare = useReport("social_period_compare", params);
  const accounts = useReport("social_by_account", params);
  const prior = before(span.from, span.to);
  const followers = (accounts.data?.rows ?? []).reduce((n, r) => n + Number(r.followers ?? 0), 0);
  const figure = (m: string) => compare.data?.rows.find((r) => r.measure === m);
  const card = (m: string, label: string) => {
    const r = figure(m);
    return <Stat label={label} value={r ? formatDecimal(Number(r.this_period), 0) : "–"} hint={r ? `${t("Before: {n}", { n: formatDecimal(Number(r.period_before), 0) })} ${changeText(r.change_pct)}`.trim() : undefined} />;
  };
  return (
    <PageStack>
      <Nav />
      <PageHead title={t("Numbers")} description={t("What your posts did, against the period before.")} actions={brand ? <a className="btn btn-default btn-md" href={`/api/v1/SocialBrand/${brand}/print?template=social.brand_report`} target="_blank" rel="noreferrer noopener">{t("The brand's week as a document")}</a> : undefined} />
      <div className="row">
        <div className="segmented" role="group" aria-label={t("Period")}>
          {[7, 28, 90].map((n) => <button key={n} type="button" aria-pressed={!custom && days === n} onClick={() => { setCustom(null); setDays(n); }}>{t("Last {n} days", { n: String(n) })}</button>)}
        </div>
        <Field label={t("From")} htmlFor="nb-from"><input id="nb-from" className="input" type="date" value={span.from} max={span.to} onChange={(e) => setCustom({ from: e.target.value, to: span.to })} /></Field>
        <Field label={t("To")} htmlFor="nb-to"><input id="nb-to" className="input" type="date" value={span.to} min={span.from} onChange={(e) => setCustom({ from: span.from, to: e.target.value })} /></Field>
        <Field label={t("Brand")} htmlFor="nb-brand"><select id="nb-brand" className="select" value={brand} onChange={(e) => setBrand(e.target.value)}><option value="">{t("All brands")}</option>{(brands.data ?? []).map((b: Row) => <option key={b.id} value={b.id}>{isolate(text(b.brand_name))}</option>)}</select></Field>
      </div>
      <p className="muted">{t("Against {from} to {to}", { from: formatMoment(`${prior.from}T12:00:00Z`, { dateStyle: "medium" }), to: formatMoment(`${prior.to}T12:00:00Z`, { dateStyle: "medium" }) })}</p>
      <Await q={compare} skeleton={<ListSkeleton rows={2} />}>
        {(r) =>
          r.rows.every((x) => !Number(x.this_period) && !Number(x.period_before)) ? (
            <EmptyState title={t("Nothing went out in this period")}>{t("Numbers show once posts are out and the networks have told how they did.")}</EmptyState>
          ) : (
            <StatGrid>
              <Stat label={t("Followers")} value={formatDecimal(followers, 0)} />
              {card("impressions", t("Impressions"))}
              {card("engagements", t("Engagements"))}
              {card("clicks", t("Clicks"))}
              {card("publications", t("Posts out"))}
            </StatGrid>
          )
        }
      </Await>
      <Followers params={params} />
      <Table name="social_by_network" title={t("By network")} params={params} />
      <Table name="social_by_account" title={t("By account")} params={params} />
      <Table name="social_by_kind" title={t("By kind of post")} params={params} />
      <Table name="social_top_posts" title={t("Top posts")} params={{ ...params, by, limit: "10" }}
        extra={<select className="select" aria-label={t("Top posts by")} value={by} onChange={(e) => setBy(e.target.value)}><option value="engagements">{t("By engagements")}</option><option value="impressions">{t("By impressions")}</option><option value="clicks">{t("By clicks")}</option></select>} />
      <Table name="social_best_times" title={t("Best times to post")} params={params} />
    </PageStack>
  );
}
