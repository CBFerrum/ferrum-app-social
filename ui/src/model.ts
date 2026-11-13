// What a network takes and asks (`meta.social`, as the account's `rules` action gives it, defaults filled) and the pure
// parts of the composer built on it: how a text is counted, which settings apply, what a post's words become with its
// signature and variables, and the live checks. No network is named here; the server counts and checks the same way
// when a post is saved (src/rules.rs), and its words win over these.

import { t } from "@ferrum/client";

export interface TextRule {
  max?: number | null;
  min?: number | null;
  count: "characters" | "graphemes" | "utf16" | "bytes" | "weighted";
  link_counts?: number | null;
  format?: string | null;
}
export interface Medium {
  max_bytes?: number | null;
  types: string[];
  min_width?: number | null;
  max_width?: number | null;
  min_height?: number | null;
  max_height?: number | null;
  min_ratio?: number | null;
  max_ratio?: number | null;
  min_seconds?: number | null;
  max_seconds?: number | null;
}
export interface MediaRule {
  kinds: string[];
  required: boolean;
  min?: number | null;
  max?: number | null;
  mixes: Record<string, number>[];
  image?: Medium | null;
  video?: Medium | null;
  document?: Medium | null;
  other: Record<string, Medium>;
  alt: boolean;
  alt_required: boolean;
  alt_max?: number | null;
  thumbnail: boolean;
}
export interface FollowRule {
  as: "thread" | "comment" | "none";
  media: boolean;
  max?: number | null;
  text?: TextRule | null;
}
export interface Kind {
  name: string;
  label?: string | null;
  text?: TextRule | null;
  media?: MediaRule | null;
  follow_ups?: FollowRule | null;
  first_comment?: boolean | null;
}
export type Option = string | { value: unknown; label: string };
export interface Setting {
  key: string;
  label: string;
  type: "text" | "long" | "number" | "bool" | "select" | "multi" | "date";
  options: Option[];
  choices?: unknown;
  required: boolean;
  default?: unknown;
  when: Record<string, unknown>;
  help?: string | null;
  min?: number | null;
  max?: number | null;
}
export interface Rules {
  name?: string | null;
  text: TextRule;
  media: MediaRule;
  kinds: Kind[];
  follow_ups: FollowRule;
  settings: Setting[];
  operations: Record<string, string>;
  accounts?: unknown;
  mentions?: unknown;
  first_comment: boolean;
  link_preview: boolean;
  scheduling: boolean;
}

/** What nothing says: a plain text of any length, no media (rules.rs `Rules::default`). */
export const NO_RULES: Rules = {
  text: { count: "characters" },
  media: { kinds: [], required: false, mixes: [], other: {}, alt: true, alt_required: false, thumbnail: false },
  kinds: [],
  follow_ups: { as: "none", media: false },
  settings: [],
  operations: {},
  first_comment: false,
  link_preview: false,
  scheduling: false,
};

// ── counting (the same as rules.rs `length`) ──

const extendsPrev = (c: number) =>
  (c >= 0x300 && c <= 0x36f) || (c >= 0x1ab0 && c <= 0x1aff) || (c >= 0x1dc0 && c <= 0x1dff) || (c >= 0x20d0 && c <= 0x20ff) || (c >= 0xfe20 && c <= 0xfe2f) ||
  (c >= 0xfe00 && c <= 0xfe0f) || (c >= 0xe0100 && c <= 0xe01ef) || c === 0x200d || (c >= 0x1f3fb && c <= 0x1f3ff) || (c >= 0xe0020 && c <= 0xe007f);

/** What a reader sees as one character: an emoji with its joiners and skin tone, a flag pair, a letter with its accents. */
export function clusters(text: string): string[] {
  const out: string[] = [];
  let start = 0;
  let prev: number | null = null;
  let regional = false;
  let i = 0;
  for (const ch of text) {
    const c = ch.codePointAt(0)!;
    const ri = c >= 0x1f1e6 && c <= 0x1f1ff;
    const joins: boolean = prev !== null && (extendsPrev(c) || prev === 0x200d || (ri && regional));
    if (!joins && i > 0) {
      out.push(text.slice(start, i));
      start = i;
    }
    regional = ri && !(joins && regional);
    prev = c;
    i += ch.length;
  }
  if (start < text.length) out.push(text.slice(start));
  return out;
}

const light = (c: number) => c <= 4351 || (c >= 8192 && c <= 8205) || (c >= 8208 && c <= 8223) || (c >= 8242 && c <= 8247);
const bytes = (s: string) => new TextEncoder().encode(s).length;

/** The `http(s)://` addresses of a text, as [start, end) in its units. */
function links(text: string): [number, number][] {
  const out: [number, number][] = [];
  let at = 0;
  for (;;) {
    const i = text.indexOf("http", at);
    if (i < 0) break;
    const rest = text.slice(i);
    if (rest.startsWith("https://") || rest.startsWith("http://")) {
      const w = rest.search(/\s/);
      const end = w < 0 ? text.length : i + w;
      out.push([i, end]);
      at = end;
    } else at = i + 4;
  }
  return out;
}

/** A text's length as the network counts it. */
export function length(rule: Pick<TextRule, "count" | "link_counts">, text: string): number {
  const unit = (s: string) => {
    switch (rule.count) {
      case "graphemes":
        return clusters(s).length;
      case "utf16":
        return s.length;
      case "bytes":
        return bytes(s);
      case "weighted":
        return clusters(s).reduce((n, c) => n + ([...c].length === 1 && light(c.codePointAt(0)!) ? 1 : 2), 0);
      default:
        return [...s].length;
    }
  };
  let n = 0;
  let at = 0;
  if (rule.link_counts != null)
    for (const [s, e] of links(text)) {
      n += unit(text.slice(at, s)) + rule.link_counts;
      at = e;
    }
  return n + unit(text.slice(at));
}

/** An item's length: its words, and a space and the link when it has one. */
export const itemLength = (rule: TextRule, text: string, link: string): number =>
  length(rule, text) + (link.trim() ? (rule.link_counts ?? length(rule, link.trim())) + 1 : 0);

// ── what a kind of post is ──

export interface Effective {
  kind: Kind | null;
  text: TextRule;
  media: MediaRule;
  follow: FollowRule;
  firstComment: boolean;
}

/** The rules of kind `name` (the first kind when empty or unknown); the network's where the kind says nothing of its own. */
export function effective(r: Rules, name: string | null | undefined): Effective {
  const kind = r.kinds.find((k) => k.name === name) ?? r.kinds[0] ?? null;
  return {
    kind,
    text: kind?.text ?? r.text,
    media: kind?.media ?? r.media,
    follow: kind?.follow_ups ?? r.follow_ups,
    firstComment: kind?.first_comment ?? r.first_comment,
  };
}

export const kindLabel = (k: Kind) => k.label || k.name;

/** The value of an option, and its label. */
export const optionValue = (o: Option): unknown => (typeof o === "string" ? o : o.value);
export const optionLabel = (o: Option): string => (typeof o === "string" ? o : o.label);

// ── settings ──

const same = (a: unknown, b: unknown) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);

/** Whether a setting is asked: the post's kind is one of `when.kind`, and each other key's answer is (one of) what `when` names. */
export function applies(s: Setting, kind: string | null, given: Record<string, unknown>): boolean {
  return Object.entries(s.when).every(([k, want]) => {
    const have = k === "kind" ? kind : (given[k] ?? null);
    return Array.isArray(want) ? want.some((w) => same(w, have)) : same(want, have);
  });
}

const empty = (v: unknown) => v == null || v === "" || (Array.isArray(v) && v.length === 0);

/** The settings asked now, in the order the rules give them. */
export const asked = (r: Rules, kind: string | null, given: Record<string, unknown>) => r.settings.filter((s) => applies(s, kind, given));

/** Answers to the settings asked (a default where nothing is given); answers to settings that do not apply are dropped. */
export function answers(r: Rules, kind: string | null, given: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const s of asked(r, kind, given)) {
    const v = empty(given[s.key]) ? s.default : given[s.key];
    if (!empty(v)) out[s.key] = v;
  }
  return out;
}

/** What is wrong with the answers, a setting's own words each (required, bounds, listed choices). */
export function settingProblems(r: Rules, kind: string | null, given: Record<string, unknown>): { key: string; why: string }[] {
  const out: { key: string; why: string }[] = [];
  for (const s of asked(r, kind, given)) {
    const v = empty(given[s.key]) ? s.default : given[s.key];
    if (empty(v)) {
      if (s.required) out.push({ key: s.key, why: t("{name} is needed", { name: s.label }) });
      continue;
    }
    if (s.type === "number") {
      const n = Number(v);
      if (Number.isNaN(n)) out.push({ key: s.key, why: t("{name} is a number", { name: s.label }) });
      else if ((s.min != null && n < s.min) || (s.max != null && n > s.max)) out.push({ key: s.key, why: t("{name} is from {min} to {max}", { name: s.label, min: String(s.min ?? "…"), max: String(s.max ?? "…") }) });
    } else if (s.type === "text" || s.type === "long") {
      const n = [...String(v)].length;
      if ((s.min != null && n < s.min) || (s.max != null && n > s.max)) out.push({ key: s.key, why: t("{name} is from {min} to {max} characters", { name: s.label, min: String(s.min ?? "…"), max: String(s.max ?? "…") }) });
    } else if (s.type === "select" && s.options.length && !s.choices && !s.options.some((o) => same(optionValue(o), v))) {
      out.push({ key: s.key, why: t("{name} is one of the choices", { name: s.label }) });
    } else if (s.type === "multi" && Array.isArray(v)) {
      if ((s.min != null && v.length < s.min) || (s.max != null && v.length > s.max)) out.push({ key: s.key, why: t("{name} takes from {min} to {max} choices", { name: s.label, min: String(s.min ?? "…"), max: String(s.max ?? "…") }) });
    }
  }
  return out;
}

// ── words made final ──

/** `text` with each `{{name}}` filled; `missing` names the first with no value (the text is then left as it was). */
export function fill(text: string, values: Record<string, string>): { text: string; missing: string | null } {
  let out = "";
  let rest = text;
  for (;;) {
    const start = rest.indexOf("{{");
    if (start < 0) break;
    const end = rest.indexOf("}}", start + 2);
    if (end < 0) break;
    const name = rest.slice(start + 2, end).trim();
    if (!(name in values)) return { text, missing: name };
    out += rest.slice(0, start) + values[name];
    rest = rest.slice(end + 2);
  }
  return { text: out + rest, missing: null };
}

/** The post's first words for one account as they go out: the signature added unless the words end with it already, then the variables filled. */
export function finalText(words: string, signature: string | null, values: Record<string, string>): { text: string; missing: string | null } {
  const sig = (signature ?? "").trim();
  const signed = sig && !words.trimEnd().endsWith(sig) ? `${words.trimEnd()}\n\n${sig}` : words;
  return fill(signed, values);
}

// ── the live checks ──

export interface Piece {
  kind: string;
  type?: string;
  bytes?: number;
  width?: number;
  height?: number;
  seconds?: number;
  alt?: string;
}
export interface CopyItem {
  text: string;
  link: string;
  media: Piece[];
}
export interface Copy {
  kind: string | null;
  items: CopyItem[];
  firstComment: string;
  settings: Record<string, unknown>;
}

const plural = (kind: string) => (kind === "image" ? t("pictures") : kind === "video" ? t("video") : kind === "document" ? t("documents") : kind);
const size = (b: number) => (b >= 1 << 20 ? `${(b / (1 << 20)).toFixed(1)} MB` : `${Math.ceil(b / 1024)} KB`);

function mediumProblems(name: string, m: MediaRule, p: Piece): string[] {
  if (!m.kinds.includes(p.kind)) return [m.kinds.length ? t("{name} takes no {what}, only {others}", { name, what: plural(p.kind), others: m.kinds.map(plural).join(", ") }) : t("{name} takes no {what}", { name, what: plural(p.kind) })];
  const rule = p.kind === "image" ? m.image : p.kind === "video" ? m.video : p.kind === "document" ? m.document : m.other[p.kind];
  const out: string[] = [];
  if (rule) {
    if (rule.max_bytes != null && p.bytes != null && p.bytes > rule.max_bytes) out.push(t("{name} takes {what} of at most {max}, and this one is {n}", { name, what: plural(p.kind), max: size(rule.max_bytes), n: size(p.bytes) }));
    if (rule.types.length && p.type && !rule.types.includes(p.type)) out.push(t("{name} takes {what} of type {types}, and this one is {type}", { name, what: plural(p.kind), types: rule.types.join(", "), type: p.type }));
    const below = (v: number | undefined, min?: number | null) => v != null && min != null && v < min;
    const above = (v: number | undefined, max?: number | null) => v != null && max != null && v > max;
    if (below(p.width, rule.min_width) || below(p.height, rule.min_height) || above(p.width, rule.max_width) || above(p.height, rule.max_height))
      out.push(t("{name} takes {what} of other dimensions than {w}×{h}", { name, what: plural(p.kind), w: String(p.width ?? "?"), h: String(p.height ?? "?") }));
    if (p.width && p.height) {
      const ratio = p.width / p.height;
      if ((rule.min_ratio != null && ratio < rule.min_ratio - 1e-9) || (rule.max_ratio != null && ratio > rule.max_ratio + 1e-9)) out.push(t("{name} does not take a {what} of this shape ({ratio})", { name, what: p.kind, ratio: ratio.toFixed(2) }));
    }
    if (p.seconds != null && ((rule.min_seconds != null && p.seconds < rule.min_seconds) || (rule.max_seconds != null && p.seconds > rule.max_seconds)))
      out.push(t("{name} takes {what} of {min} to {max} seconds, and this one is {n}", { name, what: plural(p.kind), min: String(rule.min_seconds ?? 0), max: String(rule.max_seconds ?? "…"), n: String(Math.round(p.seconds)) }));
  }
  const alt = (p.alt ?? "").trim();
  if (m.alt && m.alt_required && !alt) out.push(t("{name} needs a description of each of its media", { name }));
  if (m.alt_max != null && [...alt].length > m.alt_max) out.push(t("{name} takes descriptions of at most {max} characters", { name, max: String(m.alt_max) }));
  return out;
}

/** What is wrong with one account's copy against its rules, in words (the server checks again, in its own, when the post is saved). */
export function problems(r: Rules, copy: Copy, name: string): string[] {
  const e = effective(r, copy.kind);
  const out: string[] = [];
  copy.items.forEach((item, i) => {
    const after = i > 0;
    if (after && e.follow.as === "none") {
      if (i === 1) out.push(t("{name} takes nothing after the post", { name }));
      return;
    }
    if (!item.text.trim() && !item.media.length && !item.link.trim()) out.push(after ? t("Item {n} is empty", { n: String(i + 1) }) : t("The post is empty"));
    if (after && item.media.length && !e.follow.media) out.push(t("{name} takes no media after the post", { name }));
    if (!after && e.media.required && !item.media.length) out.push(t("{name} takes no post without an image or a video", { name }));
    for (const p of item.media) out.push(...mediumProblems(name, e.media, p));
    const total = item.media.length;
    if (e.media.max != null && total > e.media.max) out.push(t("{name} takes at most {max} media in one item, and this one has {n}", { name, max: String(e.media.max), n: String(total) }));
    if (e.media.min != null && total > 0 && total < e.media.min) out.push(t("{name} takes at least {min} media in one item, and this one has {n}", { name, min: String(e.media.min), n: String(total) }));
    if (total && e.media.mixes.length) {
      const count: Record<string, number> = {};
      for (const p of item.media) count[p.kind] = (count[p.kind] ?? 0) + 1;
      if (!e.media.mixes.some((m) => Object.entries(count).every(([k, n]) => m[k] != null && n <= m[k]))) out.push(t("{name} does not take these media together in one item", { name }));
    }
    const rule = after ? (e.follow.text ?? e.text) : e.text;
    const n = itemLength(rule, item.text, item.link);
    if (rule.max != null && n > rule.max) out.push(after ? t("{name} takes {max} characters, and item {i} is {n}", { name, max: String(rule.max), i: String(i + 1), n: String(n) }) : t("{name} takes {max} characters, and this post is {n}", { name, max: String(rule.max), n: String(n) }));
    if (rule.min != null && n < rule.min) out.push(t("{name} takes at least {min} characters, and item {i} is {n}", { name, min: String(rule.min), i: String(i + 1), n: String(n) }));
  });
  if (e.follow.max != null && e.follow.as !== "none" && copy.items.length - 1 > e.follow.max) out.push(t("{name} takes at most {max} items after the post, and this one has {n}", { name, max: String(e.follow.max), n: String(copy.items.length - 1) }));
  if (copy.firstComment.trim() && !e.firstComment) out.push(t("{name} takes no first comment", { name }));
  out.push(...settingProblems(r, e.kind?.name ?? copy.kind, copy.settings).map((p) => p.why));
  return out;
}

/** What a follow-up is on a network, in words. */
export function followWord(as: FollowRule["as"]): string {
  return as === "thread" ? t("Thread") : as === "comment" ? t("Comment") : t("Not offered");
}
