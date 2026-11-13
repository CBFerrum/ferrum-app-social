// What each account will get: its words as they go out (signature and variables in), the length as its network counts it
// against its most, every refusal in words beside it, and a neutral card of the post (picture, name, handle, the words
// with links and tags marked, the media, the follow-ups beneath). The numbers are the page's own count while the post has
// unsaved changes, and the server's (`copies`) once it is saved as it stands.

import { type ReactNode } from "react";
import { formatDecimal, isolate, t } from "@ferrum/client";
import { Alert, Badge } from "@ferrum/kit";
import { type Row, text } from "../data";
import { Face } from "../ui";
import { kindOfMedium, type MediaState } from "../draft";
import { type Copy, effective, followWord, itemLength, problems, type Rules } from "../model";

/** One account's entry of the server's `copies` answer (src/posts.rs). */
export interface ServerCopy {
  account: string;
  account_name: string | null;
  network: string;
  kind: string | null;
  follow_ups: "thread" | "comment" | "none";
  items: { text: string; link: string | null; delay_minutes: number; length: number; max: number | null; media: { file: string; kind: string; alt: string | null; name: string }[] }[];
  first_comment: string | null;
  problems: string[];
}

const MARK = /(https?:\/\/[^\s]+|#[\p{L}\p{N}_]+|@[\p{L}\p{N}_.]+)/gu;

/** The words with links, tags and mentions marked (as the networks show them). */
export function marked(words: string): ReactNode[] {
  return words.split(MARK).map((part, i) => (i % 2 ? <mark key={i} className="mark">{part}</mark> : part));
}

export function Preview({
  account,
  rules,
  copy,
  media,
  server,
}: {
  account: Row;
  rules: Rules;
  copy: Copy;
  /** The media of the account's copy (for the grid). */
  media: MediaState[][];
  /** The server's answer for the saved post, when the draft is just as saved. */
  server?: ServerCopy;
}) {
  const e = effective(rules, copy.kind);
  const name = text(account.display_name) || text(account.account_name);
  const live = problems(rules, copy, rules.name || text(account.network) || t("This network"));
  const found = server ? server.problems : live;
  const items = server
    ? server.items.map((i) => ({ text: i.text, link: i.link ?? "", n: i.length, max: i.max }))
    : copy.items.map((i, n) => {
        const rule = n > 0 ? (e.follow.text ?? e.text) : e.text;
        return { text: i.text, link: i.link, n: itemLength(rule, i.text, i.link), max: rule.max ?? null };
      });
  const followsAs = server?.follow_ups ?? e.follow.as;
  return (
    <section className="card card-body stack" aria-label={t("Preview for {name}", { name: isolate(name) })}>
      <div className="row">
        <Face name={name} picture={text(account.picture)} />
        <div className="spacer">
          <strong>{isolate(name)}</strong>
          <div className="muted">{isolate([text(account.network), text(account.handle) && `@${text(account.handle)}`].filter(Boolean).join(" · "))}</div>
        </div>
        <Badge tone={server ? "blue" : "neutral"}>{server ? t("Counted by the server") : t("Counted here")}</Badge>
      </div>
      <ul className="stack" aria-label={t("Length")}>
        {items.map((i, n) => (
          <li key={n} className={i.max != null && i.n > i.max ? "field-error" : "muted"}>
            {n === 0 ? t("The post") : followWord(followsAs)} {n === 0 ? "" : n}: {i.max != null ? t("{n} of {max}", { n: formatDecimal(i.n, 0), max: formatDecimal(i.max, 0) }) : formatDecimal(i.n, 0)}
          </li>
        ))}
      </ul>
      {found.length > 0 && (
        <Alert tone="error">
          <ul>
            {found.map((p, i) => (
              <li key={i}>{isolate(p)}</li>
            ))}
          </ul>
        </Alert>
      )}
      {found.length === 0 && <p className="muted">{t("Nothing in the way.")}</p>}
      <div className="preview-card stack" role="group" aria-label={t("How it looks")}>
        {items.map((i, n) => (
          <div key={n} className={n === 0 ? "stack" : "stack preview-follow"}>
            <p className="preview-text">{marked(i.text)}</p>
            {i.link && <p className="preview-text">{marked(i.link)}</p>}
            {(media[n] ?? []).length > 0 && (
              <div className="preview-media">
                {media[n].map((m) =>
                  kindOfMedium(m) === "image" ? <img key={m.key} src={m.file} alt={m.alt} loading="lazy" /> : <span key={m.key} className="thumb thumb-kind">{isolate(m.name)}</span>,
                )}
              </div>
            )}
          </div>
        ))}
        {copy.firstComment.trim() && <p className="muted">{t("First comment")}: {marked(copy.firstComment)}</p>}
      </div>
    </section>
  );
}
