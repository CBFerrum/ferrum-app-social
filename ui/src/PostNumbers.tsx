// What a post did, account by account: impressions, engagements, clicks, the short links and how they were clicked, and a small
// line of the days. Calls: SocialPublication list (post = id), SocialInsight list (publication = id).

import { useQuery } from "@tanstack/react-query";
import { formatDecimal, isolate, t } from "@ferrum/client";
import { Alert, Badge, ListSkeleton, PageHead, PageStack, StatePanel } from "@ferrum/kit";
import { useParams } from "react-router";
import { record } from "./data";
import { useAccounts } from "./ui";
import { type Row, rows, text } from "./data";
import { sparkline } from "./period";
import { Await } from "./ui";

export function Line({ values, label }: { values: number[]; label: string }) {
  return values.length ? (
    <svg className="spark" viewBox="0 0 120 28" role="img" aria-label={label}>
      <polyline points={sparkline(values)} fill="none" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" strokeLinecap="round" />
    </svg>
  ) : null;
}

export function One({ p, accounts }: { p: Row; accounts: Row[] }) {
  const days = useQuery({ queryKey: ["social", "insights", p.id], queryFn: () => rows("SocialInsight", [["publication", "=", p.id]], { sort: "day asc" }).catch(() => [] as Row[]) });
  const links = Object.entries((p.short_links ?? {}) as Record<string, { url: string }>);
  const name = text(accounts.find((a) => a.id === p.account)?.account_name);
  const n = (k: string) => (p[k] == null ? "–" : formatDecimal(Number(p[k]), 0));
  return (
    <li className="card card-body stack">
      <strong>{isolate(name)}</strong>
      {p.native === true && <Badge tone="purple">{t("Made on the network")}</Badge>}
      {text(p.failure) && <Alert tone={text(p.failure).startsWith("Waiting to try again") && !text(p.failed_at) ? "warning" : "error"}>{isolate(text(p.failure))}</Alert>}
      <dl className="kv">
        <div><dt>{t("Impressions")}</dt><dd>{n("impressions")}</dd></div>
        <div><dt>{t("Engagements")}</dt><dd>{n("engagements")}</dd></div>
        <div><dt>{t("Clicks")}</dt><dd>{n("clicks")}</dd></div>
      </dl>
      <Line values={(days.data ?? []).map((d) => Number(d.impressions ?? 0))} label={t("Impressions by day")} />
      {links.length > 0 && (
        <ul aria-label={t("Short links")}>
          {links.map(([long, v]) => <li key={long} className="muted">{isolate(long)} → {isolate(v.url)}</li>)}
        </ul>
      )}
      {text(p.permalink) && <a href={text(p.permalink)} target="_blank" rel="noreferrer noopener">{t("See it on the network")}</a>}
    </li>
  );
}

export function PostNumbers({ post, accounts }: { post: string; accounts: Row[] }) {
  const q = useQuery({ queryKey: ["social", "post-numbers", post], queryFn: () => rows("SocialPublication", [["post", "=", post]]) });
  return (
    <Await q={q} skeleton={<ListSkeleton rows={2} />}>
      {(list) => (list.length === 0 ? <StatePanel kind="empty" title={t("Nothing has gone out yet")}>{t("Its numbers show here once it is out.")}</StatePanel> : <ul className="stack" aria-label={t("Numbers by account")}>{list.map((p) => <One key={p.id} p={p} accounts={accounts} />)}</ul>)}
    </Await>
  );
}

/** A post made on the network itself and read back (/native/:id): its words, where it is, and its numbers; there is no post of ours to open. */
export function NativeRoute() {
  const { id = "" } = useParams();
  const q = useQuery({ queryKey: ["social", "native", id], queryFn: () => record("SocialPublication", id) });
  const accounts = useAccounts();
  return (
    <PageStack>
      <PageHead title={t("A post made on the network")} description={t("It was made on the network itself and read back, so it has no draft here; these are its numbers.")} />
      <Await q={q} skeleton={<ListSkeleton rows={2} />}>
        {(p) => (
          <div className="stack">
            {text(p.text) && <p className="preview-text card card-body">{isolate(text(p.text))}</p>}
            <ul className="stack"><One p={p} accounts={accounts.data ?? []} /></ul>
          </div>
        )}
      </Await>
    </PageStack>
  );
}
