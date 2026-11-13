// Feeds and plugs (/automation): the generic lists and forms edit them; this page reads them together with what they did: for a
// feed, the items it read and what became of each (a post made, or why not); for a plug, its runs (done, refused, skipped, and
// the note). Calls: SocialFeed, SocialFeedItem, SocialPlug, SocialPlugRun lists.

import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router";
import { formatMoment, isolate, t } from "@ferrum/client";
import { Alert, Badge, Button, ButtonLink, ListSkeleton, PageHead, PageStack, StatePanel, type Tone } from "@ferrum/kit";
import { type Row, rows, text } from "./data";
import { Await, firstOf, Nav, useAllowed } from "./ui";

const TONES: Record<string, Tone> = { Made: "green", Done: "green", Skipped: "neutral", Refused: "amber", Failed: "red" };

export function stateWord(s: string): string {
  switch (s) {
    case "Made":
      return t("Made");
    case "Done":
      return t("Done");
    case "Skipped":
      return t("Skipped");
    case "Refused":
      return t("Refused");
    case "Failed":
      return t("Failed");
    default:
      return s;
  }
}

const when = (v: unknown) => (text(v) ? formatMoment(text(v), { dateStyle: "medium", timeStyle: "short" }) : "");

function Runs({ kind, id }: { kind: "feed" | "plug"; id: string }) {
  const q = useQuery({
    queryKey: ["social", "runs", kind, id],
    queryFn: () => (kind === "feed" ? rows("SocialFeedItem", [["feed", "=", id]], { sort: "published_at desc", limit: 50 }) : rows("SocialPlugRun", [["plug", "=", id]], { sort: "at desc", limit: 50 })),
  });
  return (
    <Await q={q} skeleton={<ListSkeleton rows={3} />}>
      {(list) =>
        list.length === 0 ? (
          <p className="muted">{kind === "feed" ? t("It has read no item yet.") : t("It has not run yet.")}</p>
        ) : (
          <ul className="stack" aria-label={t("Runs")}>
            {list.map((r) => (
              <li key={r.id} className="row">
                <Badge tone={TONES[text(r.state)] ?? "neutral"}>{stateWord(text(r.state))}</Badge>
                <span className="spacer">
                  {kind === "feed" ? isolate(text(r.title)) : isolate(text(r.note) || t("Done"))}
                  {kind === "feed" && text(r.note) && <span className="muted"> · {isolate(text(r.note))}</span>}
                </span>
                {kind === "feed" && text(r.post) && <Link to={`/compose/${text(r.post)}`}>{t("The post")}</Link>}
                <span className="muted">{when(kind === "feed" ? r.published_at : r.at)}</span>
              </li>
            ))}
          </ul>
        )
      }
    </Await>
  );
}

function Section({ kind, entity, title, empty, rowsQ }: { kind: "feed" | "plug"; entity: string; title: string; empty: string; rowsQ: Row[] }) {
  const canCreate = useAllowed(entity, "create");
  const [open, setOpen] = useState<string | null>(null);
  const nameField = kind === "feed" ? "feed_name" : "plug_name";
  return (
    <section className="card card-body stack" aria-label={title}>
      <div className="row">
        <h2 className="section-title spacer">{title}</h2>
        {canCreate && <ButtonLink to={`/e/${entity}/new`} icon="plus">{t("New")}</ButtonLink>}
      </div>
      {rowsQ.length === 0 ? (
        <StatePanel kind="empty" title={empty} />
      ) : (
        <ul className="stack">
          {rowsQ.map((r) => (
            <li key={r.id} className="stack">
              <div className="row">
                <Link className="spacer" to={`/e/${entity}/${r.id}`}>{isolate(text(r[nameField]))}</Link>
                {r.active === false && <Badge>{t("Off")}</Badge>}
                {kind === "feed" && text(r.read_at) && <span className="muted">{t("Read {at}", { at: when(r.read_at) })}</span>}
                <Button size="sm" aria-expanded={open === r.id} onClick={() => setOpen(open === r.id ? null : r.id)}>{open === r.id ? t("Hide runs") : t("Runs")}</Button>
              </div>
              {text(r.last_error) && <Alert tone="error">{isolate(text(r.last_error))}</Alert>}
              {open === r.id && <Runs kind={kind} id={r.id} />}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export function AutomationRoute() {
  const feeds = useQuery({ queryKey: ["social", "feeds"], queryFn: () => rows("SocialFeed", [], { sort: "feed_name asc" }) });
  const plugs = useQuery({ queryKey: ["social", "plugs"], queryFn: () => rows("SocialPlug", [], { sort: "plug_name asc" }) });
  return (
    <PageStack>
      <Nav />
      <PageHead title={t("Feeds and plugs")} description={t("Feeds make posts from new items; plugs act when a post reaches a number.")} />
      <Await q={firstOf(feeds, plugs)}>
        {() => (
          <>
            <Section kind="feed" entity="SocialFeed" title={t("Feeds")} empty={t("No feed yet")} rowsQ={feeds.data ?? []} />
            <Section kind="plug" entity="SocialPlug" title={t("Plugs")} empty={t("No plug yet")} rowsQ={plugs.data ?? []} />
          </>
        )}
      </Await>
    </PageStack>
  );
}
