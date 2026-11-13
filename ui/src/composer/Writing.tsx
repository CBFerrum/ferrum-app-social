// Writing help: write from a prompt, rewrite (the shipped presets or a free instruction), adapt for each account, hashtags,
// describe a picture, split into a thread, a picture from a prompt. What comes back is a proposal: taken (into the original or
// into an account's version), copied, or discarded; nothing is written over the post unasked. Hidden for a person without the
// role Social Writing Help; where the server says no connection is set, its sentence is shown (with the settings for a manager).
// Calls: SocialBrand write_new, SocialPost write / rewrite / adapt / hashtags / describe / thread / picture, SocialWritingPreset.

import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router";
import { isolate, refusal, t } from "@ferrum/client";
import { Alert, Badge, Button, Field, toast } from "@ferrum/kit";
import { useMe } from "@ferrum/records";
import { type Row, rows, run, text } from "../data";
import { kindOfMedium, type Draft, mediaIndex } from "../draft";
import { useAllowed } from "../ui";

export const ROLE = "Social Writing Help";

export interface Adapted {
  account: string;
  account_name: string;
  text: string;
  length: number;
  max: number | null;
  fits: boolean;
}

/** What the writing panel asks the composer to do with a proposal the person took. */
export interface Take {
  /** The words, into the original (account empty) or an account's version. */
  words(text: string, account: string): void;
  /** Each account's adapted words into its version. */
  versions(list: Adapted[]): void;
  /** A thread: the post's words and its follow-ups. */
  thread(items: string[]): void;
  /** Hashtags at the end of the words. */
  hashtags(tags: string[], account: string): void;
  /** A picture's description. */
  alt(key: string, alt: string): void;
  /** A generated picture. */
  picture(assetId: string): Promise<void>;
}

type Proposal = { kind: "text"; text: string } | { kind: "versions"; list: Adapted[] } | { kind: "tags"; tags: string[] } | { kind: "alt"; key: string; alt: string } | { kind: "thread"; items: string[] };

export function Writing({ postId, brandId, draft, accounts, dirty, onSave, take }: { postId: string | null; brandId: string | null; draft: Draft; accounts: Row[]; dirty: boolean; onSave: () => void; take: Take }) {
  const me = useMe();
  const manager = useAllowed("SocialSettings", "write");
  const presets = useQuery({ queryKey: ["social", "presets"], queryFn: () => rows("SocialWritingPreset").catch(() => [] as Row[]) });
  const [prompt, setPrompt] = useState("");
  const [instruction, setInstruction] = useState("");
  const [preset, setPreset] = useState("");
  const [count, setCount] = useState(5);
  const [parts, setParts] = useState(3);
  const [medium, setMedium] = useState("");
  const [drawing, setDrawing] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [refused, setRefused] = useState("");
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const [into, setInto] = useState("");
  const user = (me as { data?: { user: { roles: string[]; is_admin: boolean } } }).data?.user;
  if (user && !user.is_admin && !user.roles.includes(ROLE)) return null;
  const chosen = draft.targets.map((x) => x.account);
  const media = [draft.original, ...chosen.map((a) => draft.versions[a]).filter(Boolean)].flatMap((c) => c.items.flatMap((i) => i.media)).filter((m) => kindOfMedium(m) === "image");
  const needsSaved = !postId || dirty;
  const ask = async <T,>(name: string, params: Record<string, unknown>, then: (out: T) => void | Promise<void>, brandLevel = false) => {
    setBusy(name);
    setRefused("");
    setProposal(null);
    try {
      const out = brandLevel ? await run<T>("SocialBrand", brandId!, name, params) : await run<T>("SocialPost", postId!, name, params);
      await then(out);
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(null);
    }
  };
  const gone = refused.includes("no connection is set");
  const savedFirst = (
    <div className="row">
      <span className="muted">{t("This works on the saved post.")}</span>
      <Button size="sm" onClick={onSave}>{t("Save the draft first")}</Button>
    </div>
  );
  const accept = (to: string) => {
    if (proposal?.kind === "text") take.words(proposal.text, to);
    else if (proposal?.kind === "versions") take.versions(proposal.list);
    else if (proposal?.kind === "thread") take.thread(proposal.items);
    else if (proposal?.kind === "tags") take.hashtags(proposal.tags, to);
    else if (proposal?.kind === "alt") take.alt(proposal.key, proposal.alt);
    setProposal(null);
  };
  const copy = async () => {
    const what = proposal?.kind === "text" ? proposal.text : proposal?.kind === "tags" ? proposal.tags.join(" ") : proposal?.kind === "alt" ? proposal.alt : proposal?.kind === "thread" ? proposal.items.join("\n---\n") : proposal?.kind === "versions" ? proposal.list.map((v) => v.text).join("\n---\n") : "";
    try {
      await navigator.clipboard.writeText(what);
      toast(t("Copied"));
    } catch {
      toast(t("Copy it by hand: select the words"), "error");
    }
  };
  return (
    <section className="card card-body stack" aria-labelledby="writing-h">
      <h2 id="writing-h" className="section-title">{t("Writing help")}</h2>
      {refused && (
        <Alert tone="error">
          {isolate(refused)} {gone && manager && <Link to="/apps/social/settings">{t("Open the settings")}</Link>}
        </Alert>
      )}
      <Field label={t("Write from a prompt")} htmlFor="wh-prompt">
        <textarea id="wh-prompt" className="textarea" rows={2} value={prompt} onChange={(e) => setPrompt(e.target.value)} />
      </Field>
      <div>
        <Button size="sm" loading={busy === "write" || busy === "write_new"} disabled={!prompt.trim() || (!postId && !brandId)} onClick={() => void (postId ? ask<{ text: string }>("write", { prompt }, (o) => setProposal({ kind: "text", text: o.text })) : ask<{ text: string }>("write_new", { prompt }, (o) => setProposal({ kind: "text", text: o.text }), true))}>{t("Write it")}</Button>
      </div>
      {needsSaved ? savedFirst : (
        <>
          <div className="row">
            <Field label={t("Rewrite")} htmlFor="wh-preset">
              <select id="wh-preset" className="select" value={preset} onChange={(e) => setPreset(e.target.value)}>
                <option value="">{t("Choose a preset")}</option>
                {(presets.data ?? []).map((p) => <option key={p.id} value={p.id}>{isolate(text(p.preset_name))}</option>)}
              </select>
            </Field>
            <Field label={t("Or say how")} htmlFor="wh-instruction"><input id="wh-instruction" className="input" value={instruction} onChange={(e) => setInstruction(e.target.value)} /></Field>
            <Button size="sm" loading={busy === "rewrite"} disabled={!preset && !instruction.trim()} onClick={() => void ask<{ text: string }>("rewrite", instruction.trim() ? { instruction } : { preset }, (o) => setProposal({ kind: "text", text: o.text }))}>{t("Rewrite it")}</Button>
          </div>
          <div className="row">
            <Button size="sm" loading={busy === "adapt"} disabled={!chosen.length} onClick={() => void ask<{ versions: Adapted[] }>("adapt", {}, (o) => setProposal({ kind: "versions", list: o.versions }))}>{t("Adapt for each account")}</Button>
          </div>
          <div className="row">
            <Field label={t("How many")} htmlFor="wh-count"><input id="wh-count" className="input" type="number" min={1} max={30} value={count} onChange={(e) => setCount(Number(e.target.value))} /></Field>
            <Button size="sm" loading={busy === "hashtags"} onClick={() => void ask<{ hashtags: string[] }>("hashtags", { count }, (o) => setProposal({ kind: "tags", tags: o.hashtags }))}>{t("Suggest hashtags")}</Button>
          </div>
          <div className="row">
            <Field label={t("Parts")} htmlFor="wh-parts"><input id="wh-parts" className="input" type="number" min={2} max={25} value={parts} onChange={(e) => setParts(Number(e.target.value))} /></Field>
            <Button size="sm" loading={busy === "thread"} onClick={() => void ask<{ items: string[] }>("thread", { parts }, (o) => setProposal({ kind: "thread", items: o.items }))}>{t("Split into a thread")}</Button>
          </div>
          {media.length > 0 && (
            <div className="row">
              <Field label={t("Describe a picture")} htmlFor="wh-medium">
                <select id="wh-medium" className="select" value={medium} onChange={(e) => setMedium(e.target.value)}>
                  <option value="">{t("Choose a picture")}</option>
                  {media.map((m) => <option key={m.key} value={m.key}>{isolate(m.name)}</option>)}
                </select>
              </Field>
              <Button size="sm" loading={busy === "describe"} disabled={!medium} onClick={() => void ask<{ alt: string; medium: number }>("describe", { medium: mediaIndex(draft, medium) }, (o) => setProposal({ kind: "alt", key: medium, alt: o.alt }))}>{t("Describe it")}</Button>
            </div>
          )}
          <div className="row">
            <Field label={t("A picture from a prompt")} htmlFor="wh-draw"><input id="wh-draw" className="input" value={drawing} onChange={(e) => setDrawing(e.target.value)} /></Field>
            <Button size="sm" loading={busy === "picture"} disabled={!drawing.trim()} onClick={() => void ask<{ id: string }>("picture", { prompt: drawing }, async (o) => { await take.picture(o.id); toast(t("The picture is added to the post")); })}>{t("Make a picture")}</Button>
          </div>
        </>
      )}
      {proposal && (
        <div className="card card-body stack" role="group" aria-label={t("What came back")}>
          <Badge tone="blue">{t("A proposal: nothing is changed until you take it")}</Badge>
          {proposal.kind === "text" && <p className="preview-text">{isolate(proposal.text)}</p>}
          {proposal.kind === "tags" && <p className="preview-text">{isolate(proposal.tags.join(" "))}</p>}
          {proposal.kind === "alt" && <p className="preview-text">{isolate(proposal.alt)}</p>}
          {proposal.kind === "thread" && <ol className="stack">{proposal.items.map((x, i) => <li key={i} className="preview-text">{isolate(x)}</li>)}</ol>}
          {proposal.kind === "versions" && (
            <ul className="stack" aria-label={t("Before and after")}>
              {proposal.list.map((v) => {
                const before = draft.versions[v.account]?.items[0].text ?? draft.original.items[0].text;
                return (
                  <li key={v.account} className="stack">
                    <strong>{isolate(v.account_name)}</strong> {v.max != null && <span className={v.fits ? "muted" : "field-error"}>{t("{n} of {max}", { n: String(v.length), max: String(v.max) })}</span>}
                    <p className="muted preview-text">{t("Before")}: {isolate(before)}</p>
                    <p className="preview-text">{t("After")}: {isolate(v.text)}</p>
                  </li>
                );
              })}
            </ul>
          )}
          <div className="row">
            {proposal.kind === "versions" ? (
              <Button size="sm" variant="primary" onClick={() => accept("")}>{t("Put each in its account's version")}</Button>
            ) : proposal.kind === "alt" || proposal.kind === "thread" ? (
              <Button size="sm" variant="primary" onClick={() => accept("")}>{proposal.kind === "alt" ? t("Use as the description") : t("Use as the thread")}</Button>
            ) : (
              <>
                <Button size="sm" variant="primary" onClick={() => accept("")}>{proposal.kind === "tags" ? t("Add to the original") : t("Replace the original's words")}</Button>
                {chosen.length > 0 && (
                  <>
                    <select className="select" aria-label={t("Into a version")} value={into} onChange={(e) => setInto(e.target.value)}>
                      <option value="">{t("Into a version…")}</option>
                      {chosen.map((a) => <option key={a} value={a}>{isolate(text(accounts.find((x) => x.id === a)?.account_name))}</option>)}
                    </select>
                    <Button size="sm" disabled={!into} onClick={() => accept(into)}>{t("Take into that version")}</Button>
                  </>
                )}
              </>
            )}
            <Button size="sm" onClick={() => void copy()}>{t("Copy")}</Button>
            <Button size="sm" onClick={() => setProposal(null)}>{t("Discard")}</Button>
          </div>
        </div>
      )}
    </section>
  );
}
