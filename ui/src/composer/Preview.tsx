// What each account will get, as the server counts and checks it (`_calc` with want_checks for a post not yet saved, the
// same answer as `copies`): its words as they go out (signature and variables in), the length against its network's most,
// every refusal in the server's words beside the account, and a neutral card of the post (picture, name, handle, the words
// with links and tags marked, the media, the follow-ups beneath, "see more" at the rules' fold). While a newer answer is
// awaited the last one stays, dimmed; there is never a guess.

import { type ReactNode, useState } from "react";
import { formatDecimal, isolate, t } from "@ferrum/client";
import { Alert } from "@ferrum/kit";
import { type Row, text } from "../data";
import { Face } from "../ui";
import { kindOfMedium, type MediaState } from "../draft";
import { followWord } from "../model";

/** One account's entry of `copies` (src/posts.rs). */
export interface ServerCopy {
  account: string;
  account_name: string | null;
  network: string;
  kind: string | null;
  follow_ups: "thread" | "comment" | "none";
  items: { text: string; link: string | null; delay_minutes: number; length: number; max: number | null; fold?: number | null; media: { file: string; kind: string; alt: string | null; name: string }[] }[];
  first_comment: string | null;
  /** The title as it goes (round four) and the most the network takes of one. */
  title?: string | null;
  title_max?: number | null;
  problems: string[];
}

const MARK = /(https?:\/\/[^\s]+|#[\p{L}\p{N}_]+|@[\p{L}\p{N}_.]+)/gu;

/** The words with links, tags and mentions marked (as the networks show them). */
export function marked(words: string): ReactNode[] {
  return words.split(MARK).map((part, i) => (i % 2 ? <mark key={i} className="mark">{part}</mark> : part));
}

/** The words up to the fold (in characters), and whether anything is left behind "see more". */
export function folded(words: string, fold: number | null | undefined): { shown: string; more: boolean } {
  const chars = [...words];
  return fold != null && chars.length > fold ? { shown: chars.slice(0, fold).join(""), more: true } : { shown: words, more: false };
}

function Words({ words, fold }: { words: string; fold?: number | null }) {
  const [open, setOpen] = useState(false);
  const { shown, more } = folded(words, fold);
  return (
    <p className="preview-text">
      {marked(open || !more ? words : shown)}
      {more && (
        <>
          {!open && "… "}
          <button type="button" className="link-btn" onClick={() => setOpen(!open)}>{open ? t("See less") : t("See more")}</button>
        </>
      )}
    </p>
  );
}

export function Preview({ account, copy, media, dimmed }: { account: Row; copy?: ServerCopy; media: MediaState[][]; dimmed: boolean }) {
  const name = text(account.display_name) || text(account.account_name);
  return (
    <section className="card card-body stack" data-dimmed={dimmed || undefined} aria-busy={dimmed || undefined} aria-label={t("Preview for {name}", { name: isolate(name) })}>
      <div className="row">
        <Face name={name} picture={text(account.picture)} />
        <div className="spacer">
          <strong>{isolate(name)}</strong>
          <div className="muted">{isolate([text(account.network), text(account.handle) && `@${text(account.handle)}`].filter(Boolean).join(" · "))}</div>
        </div>
      </div>
      {!copy ? (
        <p className="muted">{t("Checking…")}</p>
      ) : (
        <>
          <ul className="stack" aria-label={t("Length")}>
            {copy.items.map((i, n) => (
              <li key={n} className={i.max != null && i.length > i.max ? "field-error" : "muted"}>
                {n === 0 ? t("The post") : `${followWord(copy.follow_ups)} ${n}`}: {i.max != null ? t("{n} of {max}", { n: formatDecimal(i.length, 0), max: formatDecimal(i.max, 0) }) : formatDecimal(i.length, 0)}
              </li>
            ))}
          </ul>
          {copy.problems.length > 0 ? (
            <Alert tone="error">
              <ul>
                {copy.problems.map((p, i) => (
                  <li key={i}>{isolate(p)}</li>
                ))}
              </ul>
            </Alert>
          ) : (
            <p className="muted">{t("Nothing in the way.")}</p>
          )}
          {copy.title_max != null && (
            <p className={[...(copy.title ?? "").trim()].length > copy.title_max ? "field-error" : "muted"}>{t("Title: {n} of {max}", { n: String([...(copy.title ?? "").trim()].length), max: String(copy.title_max) })}</p>
          )}
          <div className="preview-card stack" role="group" aria-label={t("How it looks")}>
            {copy.title?.trim() && <strong className="preview-text">{isolate(copy.title)}</strong>}
            {copy.items.map((i, n) => (
              <div key={n} className={n === 0 ? "stack" : "stack preview-follow"}>
                <Words words={i.text} fold={i.fold} />
                {i.link && <p className="preview-text">{marked(i.link)}</p>}
                {(media[n] ?? []).length > 0 && (
                  <div className="preview-media">
                    {media[n].map((m) => (kindOfMedium(m) === "image" ? <img key={m.key} src={m.file} alt={m.alt} loading="lazy" /> : <span key={m.key} className="thumb thumb-kind">{isolate(m.name)}</span>))}
                  </div>
                )}
              </div>
            ))}
            {copy.first_comment?.trim() && <p className="muted">{t("First comment")}: {marked(copy.first_comment)}</p>}
          </div>
        </>
      )}
    </section>
  );
}
