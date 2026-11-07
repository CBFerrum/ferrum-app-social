// Social's home: what waits for approval, the posts coming by day, the inbox, and what each brand's
// posts did (impressions and engagements, read back through the connections), with the report of
// what each day added. Each opens its record.

import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";
import { api, docPath, t as tr, isolate } from "@ferrum/client";
import { Badge, ErrorState, PageLoading, useShellTitle } from "@ferrum/kit";

export interface Row {
  id: string;
  [k: string]: unknown;
}

const list = (entity: string, filters: unknown[] = [], extra: Record<string, string | number> = {}) =>
  api.get<{ rows: Row[] }>(docPath(entity), { filters: JSON.stringify(filters), limit: 500, ...extra }).then((r) => r.rows);

/** Posts by the day they go out (UTC), days in order. */
export function byDay(posts: Row[]): [string, Row[]][] {
  const out = new Map<string, Row[]>();
  for (const p of posts) {
    const day = typeof p.scheduled_at === "string" ? p.scheduled_at.slice(0, 10) : "";
    out.set(day, [...(out.get(day) ?? []), p]);
  }
  return [...out.entries()].sort(([a], [b]) => a.localeCompare(b));
}

/** Engagements per impression, as a percentage with one decimal, or a dash without impressions. */
export function rate(impressions: number, engagements: number): string {
  return impressions > 0 ? `${((engagements / impressions) * 100).toFixed(1)}%` : "–";
}

/** Each brand's published posts, impressions and engagements, from its publications. */
export function totals(publications: Row[], accounts: Row[]): Map<string, { posts: Set<string>; impressions: number; engagements: number }> {
  const brandOf = new Map(accounts.map((a) => [a.id, String(a.brand)]));
  const out = new Map<string, { posts: Set<string>; impressions: number; engagements: number }>();
  for (const p of publications) {
    const brand = brandOf.get(String(p.account));
    if (!brand) continue;
    const t = out.get(brand) ?? { posts: new Set<string>(), impressions: 0, engagements: 0 };
    if (p.post) t.posts.add(String(p.post));
    t.impressions += Number(p.impressions ?? 0);
    t.engagements += Number(p.engagements ?? 0);
    out.set(brand, t);
  }
  return out;
}

export function Home() {
  useShellTitle(tr("Social"));
  const waiting = useQuery({ queryKey: ["social", "waiting"], queryFn: () => list("SocialPost", [["status", "=", "In Review"]], { sort: "scheduled_at asc" }) });
  const coming = useQuery({ queryKey: ["social", "coming"], queryFn: () => list("SocialPost", [["status", "in", ["Approved", "Sending"]]], { sort: "scheduled_at asc" }) });
  const inbox = useQuery({ queryKey: ["social", "inbox"], queryFn: () => list("SocialMessage", [["status", "in", ["New", "Assigned"]]]) });
  const brands = useQuery({ queryKey: ["social", "brands"], queryFn: () => list("SocialBrand", [], { sort: "brand_name asc" }) });
  const accounts = useQuery({ queryKey: ["social", "accounts"], queryFn: () => list("SocialAccount") });
  const publications = useQuery({ queryKey: ["social", "publications"], queryFn: () => list("SocialPublication") });
  const all = [waiting, coming, inbox, brands, accounts, publications];
  const failed = all.find((q) => q.isError);
  if (failed) return <ErrorState error={failed.error} onRetry={() => void failed.refetch()} />;
  if (all.some((q) => !q.data)) return <PageLoading />;
  const sums = totals(publications.data!, accounts.data!);
  return (
    <div className="np-page stack">
      <header className="row" style={{ justifyContent: "space-between" }}>
        <h2>{tr("Social")}</h2>
        <Link to="/e/SocialPost/new">{tr("New post")}</Link>
      </header>
      <section className="card card-pad stack" aria-label={tr("Waiting for approval")}>
        <h3>{tr("Waiting for approval")}</h3>
        {waiting.data!.length === 0 && <p className="faint">{tr("Nothing waits.")}</p>}
        <ul className="stack">
          {waiting.data!.map((p) => (
            <li key={p.id}>
              <Link to={`/e/SocialPost/${p.id}`}>{isolate(String(p.headline))}</Link> <span className="faint">{tr("by {value}", { value: String(p.author ?? "") })}</span>
            </li>
          ))}
        </ul>
      </section>
      <section className="card card-pad stack" aria-label={tr("Coming")}>
        <h3>{tr("Coming")}</h3>
        {coming.data!.length === 0 && <p className="faint">{tr("Nothing is scheduled.")}</p>}
        {byDay(coming.data!).map(([day, posts]) => (
          <div key={day} className="stack">
            <strong>{isolate(day)}</strong>
            <ul className="stack">
              {posts.map((p) => (
                <li key={p.id}>
                  {isolate(String(p.scheduled_at ?? "").slice(11, 16))} <Link to={`/e/SocialPost/${p.id}`}>{isolate(String(p.headline))}</Link>{isolate(" ")}
                  <Badge tone={p.status === "Sending" ? "amber" : "green"}>{isolate(String(p.status))}</Badge>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </section>
      <section className="card card-pad stack" aria-label={tr("Inbox")}>
        <header className="row" style={{ justifyContent: "space-between" }}>
          <h3>{tr("Inbox")}</h3>
          <Link to="/e/SocialMessage">{tr("All messages")}</Link>
        </header>
        {inbox.data!.length === 0 && <p className="faint">{tr("Every message is answered.")}</p>}
        <ul className="stack">
          {inbox.data!.map((m) => (
            <li key={m.id}>
              <Link to={`/e/SocialMessage/${m.id}`}>{isolate(String(m.author))}</Link>: {isolate(String(m.text ?? "").slice(0, 120))} <Badge tone={m.status === "New" ? "red" : "amber"}>{isolate(String(m.status))}</Badge>
            </li>
          ))}
        </ul>
      </section>
      <section className="card card-pad stack" aria-label={tr("What posts did")}>
        <header className="row" style={{ justifyContent: "space-between" }}>
          <h3>{tr("What posts did")}</h3>
          <span className="row" style={{ gap: 12 }}>
            <Link to="/r/social_engagement_by_day">{tr("Day by day")}</Link>
            <Link to="/r/social_best_times">{tr("Best times to post")}</Link>
          </span>
        </header>
        <table className="table">
          <thead>
            <tr>
              <th>{tr("Brand")}</th>
              <th>{tr("Posts out")}</th>
              <th>{tr("Impressions")}</th>
              <th>{tr("Engagements")}</th>
              <th>{tr("Rate")}</th>
            </tr>
          </thead>
          <tbody>
            {brands.data!.map((b) => {
              const t = sums.get(b.id) ?? { posts: new Set<string>(), impressions: 0, engagements: 0 };
              return (
                <tr key={b.id}>
                  <td>{isolate(String(b.brand_name))}</td>
                  <td>{t.posts.size}</td>
                  <td>{t.impressions}</td>
                  <td>{t.engagements}</td>
                  <td>{isolate(rate(t.impressions, t.engagements))}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </section>
    </div>
  );
}
