// A post in the composer: the original, a version for each account that has its own, and what each account is asked.
// The conversions to and from the record (docs/interface-notes.md section 2) are here, apart from the screen, so they
// can be tested: a version is the whole post for its account (its words, its media, its follow-ups), nothing of the
// original mixed in; an account without one gets the original.

import { fromZonedInput, toZonedInput } from "@ferrum/client";
import { answers, effective, type Rules } from "./model";
import type { Row } from "./data";

export interface MediaState {
  key: string;
  /** `/files/<id>` */
  file: string;
  name: string;
  /** As stored: empty means the file's type says. */
  kind: string;
  type?: string;
  alt: string;
  thumbnail: string;
  width?: number;
  height?: number;
  seconds?: number;
  bytes?: number;
  /** The library asset it was taken from; the server fills from it what the line leaves empty. */
  asset?: string;
}
export interface ItemState {
  key: string;
  text: string;
  link: string;
  /** Minutes after the item before (a follow-up). */
  delay: number;
  media: MediaState[];
}
export interface Content {
  /** The first is the post itself; the rest are its follow-ups, in order. */
  items: ItemState[];
}
export interface TargetState {
  account: string;
  kind: string;
  settings: Record<string, unknown>;
}
export interface Draft {
  headline: string;
  targets: TargetState[];
  original: Content;
  versions: Record<string, Content>;
  firstComment: string;
  addSignature: boolean;
  signature: string;
  /** `YYYY-MM-DDTHH:mm` on the workspace's clock; empty: no time. */
  at: string;
  repeatEvery: string;
  repeatUnit: "Days" | "Weeks" | "Months";
  repeatUntil: string;
  repeatCount: string;
  tags: string[];
}

let counter = 0;
export const keyed = () => `k${++counter}`;

export const blankItem = (): ItemState => ({ key: keyed(), text: "", link: "", delay: 0, media: [] });
export const emptyDraft = (): Draft => ({
  headline: "",
  targets: [],
  original: { items: [blankItem()] },
  versions: {},
  firstComment: "",
  addSignature: false,
  signature: "",
  at: "",
  repeatEvery: "",
  repeatUnit: "Days",
  repeatUntil: "",
  repeatCount: "",
  tags: [],
});

/** The kind a medium is when the file says: by its content type, else by its name. */
export function guessKind(type: string | undefined, file: string): string {
  const major = (type ?? "").split("/")[0];
  if (major === "image" || major === "video") return major;
  if (type === "application/pdf" || /\.pdf$/i.test(file)) return "document";
  if (!type) {
    if (/\.(png|jpe?g|gif|webp|avif|heic)$/i.test(file)) return "image";
    if (/\.(mp4|mov|webm|m4v)$/i.test(file)) return "video";
  }
  return "file";
}
export const kindOfMedium = (m: MediaState) => (m.kind.trim() ? m.kind.trim().toLowerCase() : guessKind(m.type, m.file));

const clone = (c: Content): Content => ({ items: c.items.map((i) => ({ ...i, key: keyed(), media: i.media.map((m) => ({ ...m, key: keyed() })) })) });

/** The account gets a version of its own: a copy of the original to change. */
export const customise = (d: Draft, account: string): Draft => (d.versions[account] ? d : { ...d, versions: { ...d.versions, [account]: clone(d.original) } });

/** Back to the original: the account's own rows go. */
export function dropVersion(d: Draft, account: string): Draft {
  const { [account]: _gone, ...rest } = d.versions;
  return { ...d, versions: rest };
}

export function removeAccount(d: Draft, account: string): Draft {
  return { ...dropVersion(d, account), targets: d.targets.filter((t) => t.account !== account) };
}

/** What an account gets: its version, else the original. */
export const contentFor = (d: Draft, account: string): Content => d.versions[account] ?? d.original;

const num = (v: unknown): number | undefined => (typeof v === "number" && Number.isFinite(v) ? v : typeof v === "string" && v.trim() && !Number.isNaN(Number(v)) ? Number(v) : undefined);
const str = (v: unknown) => (typeof v === "string" ? v : "");
const accountOf = (r: Row) => (typeof r.account === "string" && r.account ? r.account : "");
const lines = (v: unknown): Row[] => (Array.isArray(v) ? (v as Row[]) : []);

function mediaOf(r: Row): MediaState {
  const file = str(r.file);
  return { key: keyed(), file, name: file.split("/").pop() ?? file, kind: str(r.kind), alt: str(r.alt), thumbnail: str(r.thumbnail), width: num(r.width), height: num(r.height), seconds: num(r.seconds), asset: str(r.asset) || undefined };
}

/** The content one account (or the original, `account` empty) has: its words, then its follow-ups, with the media lines laid on the items they name. */
function contentOf(row: Row, account: string, words: { text: string; link: string }): Content {
  const items: ItemState[] = [{ key: keyed(), text: words.text, link: words.link, delay: 0, media: [] }];
  for (const r of lines(row.items).filter((l) => accountOf(l) === account)) items.push({ key: keyed(), text: str(r.text), link: str(r.link), delay: num(r.delay_minutes) ?? 0, media: [] });
  // 0.7.0's one picture is the original's first medium.
  if (!account && str(row.image)) items[0].media.push(mediaOf({ id: "", file: row.image, alt: row.image_alt } as Row));
  for (const r of lines(row.media).filter((l) => accountOf(l) === account)) {
    const n = Math.max(1, num(r.item) ?? 1);
    while (items.length < n) items.push(blankItem());
    items[n - 1].media.push(mediaOf(r));
  }
  return { items };
}

/** The record as a draft. */
export function fromRecord(row: Row): Draft {
  const targets = lines(row.targets);
  const d = emptyDraft();
  d.headline = str(row.headline);
  d.original = contentOf(row, "", { text: str(row.text), link: str(row.link) });
  d.firstComment = str(row.first_comment);
  d.addSignature = row.add_signature === true;
  d.signature = str(row.signature);
  d.at = toZonedInput(row.scheduled_at);
  d.repeatEvery = row.repeat_every != null ? String(row.repeat_every) : "";
  d.repeatUnit = (["Days", "Weeks", "Months"].includes(str(row.repeat_unit)) ? str(row.repeat_unit) : "Days") as Draft["repeatUnit"];
  d.repeatUntil = str(row.repeat_until).slice(0, 10);
  d.repeatCount = row.repeat_count != null ? String(row.repeat_count) : "";
  for (const t of targets) {
    const account = accountOf(t);
    if (!account) continue;
    d.targets.push({ account, kind: str(t.kind), settings: t.settings && typeof t.settings === "object" && !Array.isArray(t.settings) ? { ...(t.settings as Record<string, unknown>) } : {} });
    if (str(t.text).trim()) d.versions[account] = contentOf(row, account, { text: str(t.text), link: str(t.link) });
  }
  return d;
}

const mediaRow = (m: MediaState, item: number, account: string) => ({
  ...(account ? { account } : {}),
  item,
  file: m.file,
  ...(m.asset ? { asset: m.asset } : {}),
  kind: m.kind,
  alt: m.alt,
  thumbnail: m.thumbnail || null,
  ...(m.width ? { width: m.width } : {}),
  ...(m.height ? { height: m.height } : {}),
  ...(m.seconds != null ? { seconds: m.seconds } : {}),
});

/** The record's fields for a save: a version's rows name its account; the original's name none. 0.7.0's one picture is now media. */
export function toRecord(d: Draft, brand: string | null, rules: Record<string, Rules> = {}): Record<string, unknown> {
  const items: unknown[] = [];
  const media: unknown[] = [];
  const put = (c: Content, account: string) => {
    c.items.forEach((it, n) => {
      if (n > 0) items.push({ ...(account ? { account } : {}), text: it.text, link: it.link || null, delay_minutes: Math.max(0, Math.round(it.delay || 0)) });
      it.media.forEach((m) => media.push(mediaRow(m, n + 1, account)));
    });
  };
  put(d.original, "");
  const targets = d.targets.map((tg) => {
    const own = d.versions[tg.account];
    if (own) put(own, tg.account);
    const r = rules[tg.account];
    return {
      account: tg.account,
      kind: tg.kind,
      settings: r ? answers(r, effective(r, tg.kind).kind?.name ?? null, tg.settings) : tg.settings,
      text: own ? own.items[0].text : "",
      link: own ? own.items[0].link || null : null,
    };
  });
  const every = num(d.repeatEvery);
  return {
    headline: d.headline.trim(),
    brand,
    text: d.original.items[0].text,
    link: d.original.items[0].link.trim() || null,
    image: null,
    image_alt: null,
    first_comment: d.firstComment.trim() ? d.firstComment : null,
    add_signature: d.addSignature,
    signature: d.addSignature && d.signature ? d.signature : null,
    scheduled_at: d.at ? fromZonedInput(d.at) : null,
    repeat_every: every && every >= 1 ? every : null,
    repeat_unit: d.repeatUnit,
    repeat_until: every && d.repeatUntil ? d.repeatUntil : null,
    repeat_count: every && num(d.repeatCount) ? num(d.repeatCount) : null,
    targets,
    items,
    media,
  };
}

/** A library asset (SocialMediaAsset) as a medium of a post. */
export function mediumOfAsset(a: Row): MediaState {
  const n = (v: unknown) => (typeof v === "number" ? v : v == null || v === "" ? undefined : Number(v));
  return { key: keyed(), file: str(a.file), name: str(a.asset_name) || str(a.file), kind: str(a.kind), type: str(a.content_type) || undefined, alt: str(a.alt), thumbnail: "", width: n(a.width), height: n(a.height), seconds: n(a.seconds), bytes: n(a.bytes), asset: a.id };
}

/** The position of a medium among the media lines a save writes (the original's, then each version's in the order of the accounts): what the `describe` action's `medium` means. */
export function mediaIndex(d: Draft, key: string): number {
  let n = 0;
  const walk = (c: Content): number | null => {
    for (const it of c.items) for (const m of it.media) { if (m.key === key) return n; n++; }
    return null;
  };
  const found = walk(d.original);
  if (found !== null) return found;
  for (const tg of d.targets) {
    const own = d.versions[tg.account];
    const f = own ? walk(own) : null;
    if (f !== null) return f;
  }
  return -1;
}
