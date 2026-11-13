// The post and its follow-ups: the words of each, a link, the wait before a follow-up, and the media of each (several,
// in order, with a description and, where the network takes one, a thumbnail). Snippets and variables are inserted at
// the cursor of the item last in use. What a follow-up is (a thread post, a comment, or not offered) comes from the
// account's rules.

import { useRef, useState } from "react";
import { errorMessage, formatDecimal, isolate, t } from "@ferrum/client";
import { Alert, Button, Field, Icon } from "@ferrum/kit";
import { blankItem, type Content, type ItemState, kindOfMedium, keyed, type MediaState } from "../draft";
import { acceptOf, uploadMedium, uploadThumbnail } from "./media";
import { itemLength, type MediaRule, type TextRule } from "../model";

export interface Insertable {
  label: string;
  text: string;
}

function Medium({ m, rule, locked, first, last, onChange, onMove, onRemove }: { m: MediaState; rule: MediaRule | null; locked?: boolean; first: boolean; last: boolean; onChange: (m: MediaState) => void; onMove: (by: -1 | 1) => void; onRemove: () => void }) {
  const kind = kindOfMedium(m);
  const thumb = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState("");
  const showAlt = !rule || rule.alt;
  const facts = [m.width && m.height ? `${m.width}×${m.height}` : "", m.seconds != null ? t("{n} seconds", { n: formatDecimal(m.seconds, 0) }) : "", m.bytes != null ? `${formatDecimal(m.bytes / 1024, 0)} KB` : ""].filter(Boolean).join(" · ");
  return (
    <li className="card card-body stack">
      <div className="row">
        {kind === "image" ? <img className="thumb" src={m.file} alt="" loading="lazy" /> : <span className="thumb thumb-kind"><Icon name={kind === "video" ? "video" : "file"} /></span>}
        <div className="spacer">
          <strong>{isolate(m.name)}</strong>
          <div className="muted">{isolate([kind, facts].filter(Boolean).join(" · "))}</div>
        </div>
        {!locked && (
          <>
            <Button size="sm" icon="up" aria-label={t("Move {name} earlier", { name: isolate(m.name) })} disabled={first} onClick={() => onMove(-1)} />
            <Button size="sm" icon="down" aria-label={t("Move {name} later", { name: isolate(m.name) })} disabled={last} onClick={() => onMove(1)} />
            <Button size="sm" variant="danger" icon="trash" aria-label={t("Remove {name}", { name: isolate(m.name) })} onClick={onRemove} />
          </>
        )}
      </div>
      {showAlt && (
        <Field label={t("Description of {name}", { name: isolate(m.name) })} required={rule?.alt_required} hint={rule?.alt_max ? t("At most {n} characters.", { n: String(rule.alt_max) }) : t("What it shows, for people who cannot see it.")}>
          <input className="input" value={m.alt} disabled={locked} onChange={(e) => onChange({ ...m, alt: e.target.value })} />
        </Field>
      )}
      {rule?.thumbnail && kind === "video" && (
        <div className="row">
          {m.thumbnail && <img className="thumb" src={m.thumbnail} alt="" />}
          <input ref={thumb} type="file" accept="image/*" hidden onChange={async (e) => {
            const f = e.target.files?.[0];
            e.target.value = "";
            if (!f) return;
            setBusy(true);
            setFailed("");
            try {
              onChange({ ...m, thumbnail: await uploadThumbnail(f) });
            } catch (x) {
              setFailed(errorMessage(x));
            } finally {
              setBusy(false);
            }
          }} />
          <Button size="sm" loading={busy} disabled={locked} onClick={() => thumb.current?.click()}>
            {m.thumbnail ? t("Change the thumbnail") : t("Add a thumbnail")}
          </Button>
          {failed && <span className="field-error">{failed}</span>}
        </div>
      )}
    </li>
  );
}

export function Items({
  content,
  onChange,
  followLabel,
  followNone,
  text,
  media,
  snippets,
  variables,
  locked,
  idPrefix,
}: {
  content: Content;
  onChange: (c: Content) => void;
  /** What a follow-up is on the account this is for, in words; empty: the original's, for several accounts. */
  followLabel: string;
  /** The account takes no follow-ups. */
  followNone: boolean;
  /** The length rule of the account this is for (the original has none: each account's counter is in its checks). */
  text: TextRule | null;
  media: MediaRule | null;
  snippets: Insertable[];
  variables: string[];
  locked?: boolean;
  idPrefix: string;
}) {
  const areas = useRef<Record<string, HTMLTextAreaElement | null>>({});
  const lastUsed = useRef(0);
  const [uploading, setUploading] = useState<number | null>(null);
  const [failed, setFailed] = useState("");
  const set = (i: number, patch: Partial<ItemState>) => onChange({ items: content.items.map((it, n) => (n === i ? { ...it, ...patch } : it)) });
  const insert = (what: string) => {
    const i = Math.min(lastUsed.current, content.items.length - 1);
    const el = areas.current[content.items[i].key];
    const words = content.items[i].text;
    const [a, b] = el ? [el.selectionStart ?? words.length, el.selectionEnd ?? words.length] : [words.length, words.length];
    set(i, { text: words.slice(0, a) + what + words.slice(b) });
    requestAnimationFrame(() => {
      el?.focus();
      el?.setSelectionRange(a + what.length, a + what.length);
    });
  };
  const add = async (i: number, files: FileList | null) => {
    if (!files?.length) return;
    setUploading(i);
    setFailed("");
    try {
      const added: MediaState[] = [];
      for (const f of Array.from(files)) added.push(await uploadMedium(f));
      set(i, { media: [...content.items[i].media, ...added] });
    } catch (e) {
      setFailed(errorMessage(e));
    } finally {
      setUploading(null);
    }
  };
  const moveMedium = (i: number, j: number, by: -1 | 1) => {
    const list = [...content.items[i].media];
    [list[j], list[j + by]] = [list[j + by], list[j]];
    set(i, { media: list });
  };
  return (
    <div className="stack">
      {!locked && (snippets.length > 0 || variables.length > 0) && (
        <div className="row">
          {snippets.length > 0 && (
            <select className="select" aria-label={t("Insert a snippet")} value="" onChange={(e) => e.target.value && insert(e.target.value)}>
              <option value="">{t("Insert a snippet")}</option>
              {snippets.map((s, i) => (
                <option key={i} value={s.text}>
                  {isolate(s.label)}
                </option>
              ))}
            </select>
          )}
          {variables.length > 0 && (
            <select className="select" aria-label={t("Insert a variable")} value="" onChange={(e) => e.target.value && insert(`{{${e.target.value}}}`)}>
              <option value="">{t("Insert a variable")}</option>
              {variables.map((v) => (
                <option key={v} value={v}>
                  {isolate(v)}
                </option>
              ))}
            </select>
          )}
        </div>
      )}
      {failed && <Alert tone="error">{failed}</Alert>}
      {content.items.map((it, i) => {
        const n = itemLength(text ?? { count: "characters" }, it.text, it.link);
        const over = text?.max != null && n > text.max;
        const label = i === 0 ? t("The post") : followLabel ? t("{word} {n}", { word: followLabel, n: String(i) }) : t("Follow-up {n}", { n: String(i) });
        const id = `${idPrefix}-item-${i}`;
        const fileInput = `${id}-file`;
        return (
          <fieldset key={it.key} className="card card-body stack" disabled={locked}>
            <legend className="field-label">{label}</legend>
            {i > 0 && followNone && <Alert tone="warning">{t("This network takes nothing after the post.")}</Alert>}
            <Field label={isolate(label)} htmlFor={id} error={over ? t("{n} is over the {max} this network takes", { n: String(n), max: String(text?.max) }) : undefined}>
              <textarea id={id} className="textarea" rows={i === 0 ? 5 : 3} ref={(el) => { areas.current[it.key] = el; }} value={it.text} onFocus={() => { lastUsed.current = i; }} onChange={(e) => set(i, { text: e.target.value })} />
            </Field>
            <div className="row">
              <Field label={t("Link")} htmlFor={`${id}-link`}>
                <input id={`${id}-link`} className="input" type="url" inputMode="url" value={it.link} onFocus={() => { lastUsed.current = i; }} onChange={(e) => set(i, { link: e.target.value })} />
              </Field>
              {i > 0 && (
                <Field label={t("Wait (minutes)")} htmlFor={`${id}-delay`} hint={t("After the item before.")}>
                  <input id={`${id}-delay`} className="input" type="number" min={0} value={it.delay} onChange={(e) => set(i, { delay: Math.max(0, Number(e.target.value) || 0) })} />
                </Field>
              )}
              {text && <span className={over ? "field-error" : "muted"} aria-live="polite">{text.max != null ? t("{n} of {max}", { n: formatDecimal(n, 0), max: formatDecimal(text.max, 0) }) : formatDecimal(n, 0)}</span>}
            </div>
            {it.media.length > 0 && (
              <ul className="stack" aria-label={t("Media of {label}", { label: isolate(label) })}>
                {it.media.map((m, j) => (
                  <Medium key={m.key} m={m} rule={media} locked={locked} first={j === 0} last={j === it.media.length - 1} onChange={(next) => set(i, { media: it.media.map((x) => (x.key === m.key ? next : x)) })} onMove={(by) => moveMedium(i, j, by)} onRemove={() => set(i, { media: it.media.filter((x) => x.key !== m.key) })} />
                ))}
              </ul>
            )}
            <div className="row">
              <input id={fileInput} type="file" multiple hidden accept={acceptOf(media?.kinds ?? [])} onChange={(e) => { void add(i, e.target.files); e.target.value = ""; }} />
              <Button size="sm" icon="upload" loading={uploading === i} onClick={() => document.getElementById(fileInput)?.click()}>
                {t("Add media")}
              </Button>
              <span className="spacer" />
              {i > 0 && (
                <Button size="sm" variant="danger" icon="trash" onClick={() => onChange({ items: content.items.filter((_, n) => n !== i) })}>
                  {t("Remove")}
                </Button>
              )}
            </div>
          </fieldset>
        );
      })}
      {!locked && (
        <div>
          <Button size="sm" icon="plus" onClick={() => onChange({ items: [...content.items, { ...blankItem(), key: keyed() }] })}>
            {followLabel ? t("Add to the {word}", { word: followLabel.toLowerCase() }) : t("Add a follow-up")}
          </Button>
        </div>
      )}
    </div>
  );
}
