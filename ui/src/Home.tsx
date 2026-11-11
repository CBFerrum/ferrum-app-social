// Social's home: what waits for approval, the posts coming by day, the inbox, and what each brand's
// posts did (impressions and engagements, read back through the connections), with the report of
// what each day added. Each opens its record.

import { useQuery } from "@tanstack/react-query";
import { api, docPath, formatDecimal, t as tr, isolate } from "@ferrum/client";
import { ButtonLink, HomeSkeleton, LoadFailed, PageHead, Queue, QueueGrid, QueueItem, StatePanel, StatusBadge } from "@ferrum/kit";

export interface Row {
  id: string;
  [k: string]: unknown;
}

// SocialPost.status and SocialMessage.status as stored (packages/social/entities); the lists below show only these.
const POST_TONES = { Approved: "green", Sending: "amber" } as const;
const MESSAGE_TONES = { New: "red", Assigned: "amber" } as const;

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

/** The most a queue lists before "and N more" opens the list. */
const LIMIT = 5;

export function Home() {
  const waiting = useQuery({ queryKey: ["social", "waiting"], queryFn: () => list("SocialPost", [["status", "=", "In Review"]], { sort: "scheduled_at asc" }) });
  const coming = useQuery({ queryKey: ["social", "coming"], queryFn: () => list("SocialPost", [["status", "in", ["Approved", "Sending"]]], { sort: "scheduled_at asc" }) });
  const inbox = useQuery({ queryKey: ["social", "inbox"], queryFn: () => list("SocialMessage", [["status", "in", ["New", "Assigned"]]]) });
  const brands = useQuery({ queryKey: ["social", "brands"], queryFn: () => list("SocialBrand", [], { sort: "brand_name asc" }) });
  const accounts = useQuery({ queryKey: ["social", "accounts"], queryFn: () => list("SocialAccount") });
  const publications = useQuery({ queryKey: ["social", "publications"], queryFn: () => list("SocialPublication") });
  const all = [waiting, coming, inbox, brands, accounts, publications];
  const failed = all.find((q) => q.isError);
  const head = <PageHead title={tr("Social")} actions={<ButtonLink to="/e/SocialPost/new" variant="primary" icon="plus">{tr("New post")}</ButtonLink>} />;
  if (failed) return <>{head}<LoadFailed error={failed.error} onRetry={() => void failed.refetch()} /></>;
  if (all.some((q) => !q.data)) return <>{head}<HomeSkeleton queues={3} /></>;
  if (!brands.data!.length && !waiting.data!.length && !coming.data!.length && !inbox.data!.length)
    return (
      <>
        {head}
        <StatePanel kind="empty" title={tr("No brands yet")} action={<ButtonLink to="/e/SocialBrand/new" variant="primary" icon="plus">{tr("Add a brand")}</ButtonLink>}>
          {tr("Add a brand and connect its accounts, then posts, the inbox and what posts did show here.")}
        </StatePanel>
      </>
    );
  const sums = totals(publications.data!, accounts.data!);
  const upcoming = byDay(coming.data!).flatMap(([, posts]) => posts);
  const count = (n: number) => formatDecimal(n, 0);
  return (
    <>
      {head}
      <QueueGrid>
        <Queue title={tr("Waiting for approval")} count={waiting.data!.length} tone={waiting.data!.length ? "attention" : "default"} empty={tr("Nothing waits.")} more={{ to: "/e/SocialPost", count: waiting.data!.length - LIMIT }}>
          {waiting.data!.slice(0, LIMIT).map((p) => (
            <QueueItem key={p.id} to={`/e/SocialPost/${p.id}`} title={isolate(String(p.headline))} meta={isolate(tr("by {value}", { value: String(p.author ?? "") }))} />
          ))}
        </Queue>
        <Queue title={tr("Coming")} count={coming.data!.length} empty={tr("Nothing is scheduled.")} more={{ to: "/e/SocialPost", count: upcoming.length - LIMIT }}>
          {upcoming.slice(0, LIMIT).map((p) => (
            <QueueItem
              key={p.id}
              to={`/e/SocialPost/${p.id}`}
              title={isolate(String(p.headline))}
              meta={isolate(`${String(p.scheduled_at ?? "").slice(0, 10)} · ${String(p.scheduled_at ?? "").slice(11, 16)}`)}
              trailing={<StatusBadge value={p.status} tones={POST_TONES} />}
            />
          ))}
        </Queue>
        <Queue title={tr("Inbox")} count={inbox.data!.length} link={{ to: "/e/SocialMessage", label: tr("All messages") }} empty={tr("Every message is answered.")} more={{ to: "/e/SocialMessage", count: inbox.data!.length - LIMIT }}>
          {inbox.data!.slice(0, LIMIT).map((m) => (
            <QueueItem key={m.id} to={`/e/SocialMessage/${m.id}`} title={isolate(String(m.author))} meta={isolate(String(m.text ?? "").slice(0, 120))} trailing={<StatusBadge value={m.status} tones={MESSAGE_TONES} />} />
          ))}
        </Queue>
      </QueueGrid>
      <section className="card card-pad stack mt-6 min-w-0" aria-labelledby="social-did">
        <header className="flex flex-wrap items-center justify-between gap-2">
          <h2 id="social-did" className="text-base font-semibold">{tr("What posts did")}</h2>
          <div className="flex flex-wrap gap-1">
            <ButtonLink to="/r/social_engagement_by_day">{tr("Day by day")}</ButtonLink>
            <ButtonLink to="/r/social_best_times">{tr("Best times to post")}</ButtonLink>
          </div>
        </header>
        <div className="table-wrap" tabIndex={0} role="region" aria-label={tr("What posts did")}>
          <table className="table">
            <thead>
              <tr>
                <th scope="col">{tr("Brand")}</th>
                <th scope="col" className="num">{tr("Posts out")}</th>
                <th scope="col" className="num">{tr("Impressions")}</th>
                <th scope="col" className="num">{tr("Engagements")}</th>
                <th scope="col" className="num">{tr("Rate")}</th>
              </tr>
            </thead>
            <tbody>
              {brands.data!.map((b) => {
                const t = sums.get(b.id) ?? { posts: new Set<string>(), impressions: 0, engagements: 0 };
                return (
                  <tr key={b.id}>
                    <td>{isolate(String(b.brand_name))}</td>
                    <td className="num">{count(t.posts.size)}</td>
                    <td className="num">{count(t.impressions)}</td>
                    <td className="num">{count(t.engagements)}</td>
                    <td className="num">{isolate(rate(t.impressions, t.engagements))}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </section>
    </>
  );
}
