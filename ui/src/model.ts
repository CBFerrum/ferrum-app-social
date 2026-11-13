// What a network takes and asks (`meta.social`, as the account's `rules` action gives it, defaults filled) and the display
// logic of the composer built on it: which kind a post is, which settings are asked now. Counting, final copies and checks
// are the server's (`_calc` with want_checks, `copies`); nothing of rules.rs is repeated here.

import { t } from "@ferrum/client";

export interface TextRule {
  max?: number | null;
  min?: number | null;
  count: "characters" | "graphemes" | "utf16" | "bytes" | "weighted";
  link_counts?: number | null;
  format?: string | null;
  /** Characters after which the network shows "see more" (the preview's only). */
  fold?: number | null;
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
  /** Kinds of medium the network fetches from a public address rather than taking their bytes. */
  by_address?: string[];
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
  /** A public title the network takes: needed, how long. */
  title?: { required: boolean; max?: number | null } | null;
  /** The kind a post is when none is picked, by its first medium (`none` without media). */
  kind_by_media?: Record<string, string>;
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

/** What a follow-up is on a network, in words. */
export function followWord(as: FollowRule["as"]): string {
  return as === "thread" ? t("Thread") : as === "comment" ? t("Comment") : t("Not offered");
}
