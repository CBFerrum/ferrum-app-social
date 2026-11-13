// Social's home: what waits for approval, the posts coming by day, the inbox, and what each brand's
// posts did (impressions and engagements, read back through the connections), with the report of
// what each day added. Each opens its record.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api, docPath, errorMessage, formatDecimal, t as tr, isolate } from "@ferrum/client";
import { STATE_TONES, stateWord } from "./data";
import { Nav } from "./ui";
import { Button, ButtonLink, toast, HomeSection, HomeSkeleton, LoadFailed, PageHead, PageStack, Queue, QueueGrid, QueueItem, StatePanel, StatusBadge, type Tone } from "@ferrum/kit";

export interface Row {
  id: string;
  [k: string]: unknown;
}

// SocialPost.status and SocialMessage.status as stored (packages/social/entities); the lists below show only these.
const POST_TONES: Readonly<Record<string, Tone>> = { Approved: "green", Sending: "amber", Failed: "red" };
const MESSAGE_TONES: Readonly<Record<string, Tone>> = { New: "red", Assigned: "amber" };

const list = (entity: string, filters: unknown[] = [], extra: Record<string, string | number> = {}) =>
  api.get<{ rows: Row[] }>(docPath(entity), { filters: JSON.stringify(filters), limit: 500, ...extra }).then((r) => r.rows);

/** A failed post's reason, one line per account as the server keeps it, as a single line. */
export function reasons(failure: unknown): string {
  return String(failure ?? "").split("\n").map((l) => l.trim()).filter(Boolean).join("; ");
}

/** A failed post with its reason and Retry (the server sends it again only to the accounts that did not get it). */
function FailedPost({ post }: { post: Row }) {
  const client = useQueryClient();
  const [busy, setBusy] = useState(false);
  const retry = async () => {
    setBusy(true);
    try {
      await api.post(docPath("SocialPost", post.id, "actions", "retry"), {});
      toast(tr("Sending again"));
      await client.invalidateQueries({ queryKey: ["social"] });
    } catch (e) {
      toast(errorMessage(e), "error");
    } finally {
      setBusy(false);
    }
  };
  return (
    <QueueItem
      to={`/compose/${post.id}`}
      title={isolate(String(post.headline))}
      meta={isolate(reasons(post.failure) || tr("No reason given"))}
      action={<Button size="sm" loading={busy} onClick={() => void retry()}>{tr("Retry")}</Button>}
    />
  );
}

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
  const failedPosts = useQuery({ queryKey: ["social", "failed"], queryFn: () => list("SocialPost", [["status", "=", "Failed"]], { sort: "scheduled_at asc" }) });
  const coming = useQuery({ queryKey: ["social", "coming"], queryFn: () => list("SocialPost", [["status", "in", ["Approved", "Sending"]]], { sort: "scheduled_at asc" }) });
  // What the platform's approvals ask of this person (a post In Review that they are to decide) beside the posts In Review.
  const toDecide = useQuery({ queryKey: ["approvals"], queryFn: () => api.get<{ to_decide: { entity: string; record: string; title: string; requester_name?: string | null }[] }>("/api/approvals").then((r) => r.to_decide.filter((i) => i.entity === "SocialPost")).catch(() => []) });
  const drafts = useQuery({ queryKey: ["social", "drafts"], queryFn: () => list("SocialPost", [["status", "in", ["Draft", "Rejected"]], ["scheduled_at", "is", "not set"]], { sort: "created_at desc" }) });
  const waitingFor = useQuery({ queryKey: ["social", "waiting-for"], queryFn: () => list("SocialPost", [["status", "=", "Approved"]], { sort: "scheduled_at asc" }).then((r) => r.filter((p) => String(p.waiting_for ?? "").trim())) });
  const inbox = useQuery({ queryKey: ["social", "inbox"], queryFn: () => list("SocialMessage", [["status", "in", ["New", "Assigned"]]]) });
  const brands = useQuery({ queryKey: ["social", "brands"], queryFn: () => list("SocialBrand", [], { sort: "brand_name asc" }) });
  const accounts = useQuery({ queryKey: ["social", "accounts"], queryFn: () => list("SocialAccount") });
  const publications = useQuery({ queryKey: ["social", "publications"], queryFn: () => list("SocialPublication") });
  const all = [waiting, failedPosts, coming, inbox, brands, accounts, publications, toDecide, drafts, waitingFor];
  const failed = all.find((q) => q.isError);
  const head = (
    <>
      <Nav />
      <PageHead title={tr("Social")} actions={<ButtonLink to="/compose" variant="primary" icon="plus">{tr("New post")}</ButtonLink>} />
    </>
  );
  if (failed) return <PageStack>{head}<LoadFailed error={failed.error} onRetry={() => void failed.refetch()} /></PageStack>;
  if (all.some((q) => !q.data)) return <PageStack>{head}<HomeSkeleton queues={3} /></PageStack>;
  if (!brands.data!.length && !waiting.data!.length && !failedPosts.data!.length && !coming.data!.length && !inbox.data!.length)
    return (
      <PageStack>
        {head}
        <StatePanel kind="empty" title={tr("No brands yet")} action={<ButtonLink to="/e/SocialBrand/new" variant="primary" icon="plus">{tr("Add a brand")}</ButtonLink>}>
          {tr("Add a brand and connect its accounts, then posts, the inbox and what posts did show here.")}
        </StatePanel>
      </PageStack>
    );
  const sums = totals(publications.data!, accounts.data!);
  // The posts In Review, and those the platform's approvals ask this person to decide that the list did not hold.
  const review = [...waiting.data!, ...toDecide.data!.filter((d) => !waiting.data!.some((p) => p.id === d.record)).map((d) => ({ id: d.record, headline: d.title, author: d.requester_name ?? "" }))];
  const decide = new Set(toDecide.data!.map((d) => d.record));
  const reconnect = accounts.data!.filter((a) => a.active !== false && String(a.state ?? "Connected") !== "Connected");
  const upcoming = byDay(coming.data!).flatMap(([, posts]) => posts);
  const count = (n: number) => formatDecimal(n, 0);
  return (
    <PageStack gap="lg">
      {head}
      <QueueGrid>
        {failedPosts.data!.length > 0 && (
          <Queue title={tr("Did not go out")} count={failedPosts.data!.length} tone="attention" more={{ to: "/e/SocialPost", count: failedPosts.data!.length - LIMIT }}>
            {failedPosts.data!.slice(0, LIMIT).map((p) => (
              <FailedPost key={p.id} post={p} />
            ))}
          </Queue>
        )}
        <Queue title={tr("Waiting for approval")} count={review.length} tone={review.length ? "attention" : "default"} empty={tr("Nothing waits.")} more={{ to: "/e/SocialPost", count: review.length - LIMIT }}>
          {review.slice(0, LIMIT).map((p) => (
            <QueueItem key={p.id} to={`/compose/${p.id}`} title={isolate(String(p.headline))} meta={isolate(tr("by {value}", { value: String(p.author ?? "") }))} trailing={decide.has(p.id) ? <StatusBadge value="To decide" label={tr("Yours to decide")} tones={{ "To decide": "amber" }} /> : undefined} />
          ))}
        </Queue>
        {waitingFor.data!.length > 0 && (
          <Queue title={tr("Approved, but waiting")} count={waitingFor.data!.length} tone="attention" link={{ to: "/accounts", label: tr("Accounts") }} more={{ to: "/calendar", count: waitingFor.data!.length - LIMIT }}>
            {waitingFor.data!.slice(0, LIMIT).map((p) => (
              <QueueItem key={p.id} to={`/compose/${p.id}`} title={isolate(String(p.headline))} meta={isolate(String(p.waiting_for))} />
            ))}
          </Queue>
        )}
        {reconnect.length > 0 && (
          <Queue title={tr("Accounts to reconnect")} count={reconnect.length} tone="attention" link={{ to: "/accounts", label: tr("Accounts") }} more={{ to: "/accounts", count: reconnect.length - LIMIT }}>
            {reconnect.slice(0, LIMIT).map((a) => (
              <QueueItem key={a.id} to="/accounts" title={isolate(String(a.account_name))} meta={isolate(String(a.state_note ?? ""))} trailing={<StatusBadge value={a.state} label={stateWord(String(a.state))} tones={STATE_TONES} />} />
            ))}
          </Queue>
        )}
        <Queue title={tr("Coming")} count={coming.data!.length} link={{ to: "/calendar", label: tr("Calendar") }} empty={tr("Nothing is scheduled.")} more={{ to: "/calendar", count: upcoming.length - LIMIT }}>
          {upcoming.slice(0, LIMIT).map((p) => (
            <QueueItem
              key={p.id}
              to={`/compose/${p.id}`}
              title={isolate(String(p.headline))}
              lead={isolate(String(p.scheduled_at ?? "").slice(11, 16))}
              meta={isolate(String(p.scheduled_at ?? "").slice(0, 10))}
              trailing={<StatusBadge value={p.status} tones={POST_TONES} />}
            />
          ))}
        </Queue>
        <Queue title={tr("Drafts without a time")} count={drafts.data!.length} link={{ to: "/queue", label: tr("Posting times") }} empty={tr("No draft is waiting for a time.")} more={{ to: "/e/SocialPost", count: drafts.data!.length - LIMIT }}>
          {drafts.data!.slice(0, LIMIT).map((p) => (
            <QueueItem key={p.id} to={`/compose/${p.id}`} title={isolate(String(p.headline))} meta={isolate(String(p.status))} />
          ))}
        </Queue>
        <Queue title={tr("Inbox")} count={inbox.data!.length} link={{ to: "/e/SocialMessage", label: tr("All messages") }} empty={tr("Every message is answered.")} more={{ to: "/e/SocialMessage", count: inbox.data!.length - LIMIT }}>
          {inbox.data!.slice(0, LIMIT).map((m) => (
            <QueueItem key={m.id} to={`/e/SocialMessage/${m.id}`} title={isolate(String(m.author))} meta={isolate(String(m.text ?? "").slice(0, 120))} trailing={<StatusBadge value={m.status} tones={MESSAGE_TONES} />} />
          ))}
        </Queue>
      </QueueGrid>
      <HomeSection
        title={tr("What posts did")}
        actions={
          <>
            <ButtonLink to="/r/social_engagement_by_day">{tr("Day by day")}</ButtonLink>
            <ButtonLink to="/r/social_best_times">{tr("Best times to post")}</ButtonLink>
          </>
        }
      >
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
      </HomeSection>
    </PageStack>
  );
}
