// The composer (/compose, /compose/:id; "New post" leads here): the accounts a post goes to, the original and a version for
// each account that has its own, the post and its follow-ups with their media, what each account's network asks, live counts
// and refusals and a preview per account, the library (templates, sets, snippets, variables, signature, tags), when it goes
// (draft, a time, the queue, now, repeating), and its review. Nothing is saved but on a press: the page warns before leaving
// with changes, and a refused save shows the server's words. No network is named here; every limit, kind and question is
// drawn from the rules the server gives for an account (model.ts).

import { keepPreviousData, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";
import { api, docPath, formatMoment, isolate, refusal, t } from "@ferrum/client";
import { Alert, Badge, Button, ButtonLink, confirmDialog, Dialog, Field, FormSkeleton, Menu, MenuItem, PageHead, PageSplit, PageStack, StatusBadge, TabPanel, Tabs, toast, useDebounced, useUnsavedGuard } from "@ferrum/kit";
import { change, make, POST_TONES, type Row, remove, rows, run, statusWord, text, record } from "./data";
import { blankItem, contentFor, customise, mediumOfAsset, type Draft, dropVersion, emptyDraft, fromRecord, removeAccount, toRecord } from "./draft";
import { AccountOptions } from "./composer/Settings";
import { Items, type Insertable } from "./composer/Items";
import { Pick } from "./composer/Pick";
import { Preview, type ServerCopy } from "./composer/Preview";
import { PostNumbers } from "./PostNumbers";
import { Writing, type Adapted } from "./composer/Writing";
import { ReviewPanel } from "./composer/Review";
import { type Mode, modeOf, When } from "./composer/When";
import { effective, followWord, NO_RULES, type Rules } from "./model";
import { Await, Face, firstOf, Nav, nameOf, useAccounts, useAllowed, useBrands } from "./ui";

const LOCKED = ["Sending", "Published", "Failed"];

/** What the library holds, for the pickers: a failed read is an empty list (a person may not read all of it). */
function useLibrary() {
  const list = (entity: string) => rows(entity).catch(() => [] as Row[]);
  return useQuery({
    queryKey: ["social", "library"],
    queryFn: async () => {
      const [signatures, variables, snippets, templates, sets] = await Promise.all(["SocialSignature", "SocialVariable", "SocialSnippet", "SocialTemplate", "SocialSet"].map(list));
      return { signatures, variables, snippets, templates, sets };
    },
  });
}

const snap = (d: Draft, brand: string | null) => JSON.stringify([toRecord(d, brand), d.tags]);

function SaveAsTemplate({ post, onClose }: { post: Row; onClose: () => void }) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState("");
  const save = async () => {
    setBusy(true);
    try {
      await run("SocialPost", post.id, "save_as_template", { name });
      toast(t("Saved as a template"));
      onClose();
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog open onClose={onClose} title={t("Save as template")} footer={<><Button onClick={onClose}>{t("Cancel")}</Button><Button variant="primary" loading={busy} disabled={!name.trim()} onClick={() => void save()}>{t("Save")}</Button></>}>
      {refused && <Alert tone="error">{refused}</Alert>}
      <Field label={t("Template name")} required htmlFor="tpl-name">
        <input id="tpl-name" className="input" value={name} onChange={(e) => setName(e.target.value)} />
      </Field>
    </Dialog>
  );
}

function Tags({ tags, onChange, known, locked }: { tags: string[]; onChange: (t: string[]) => void; known: string[]; locked?: boolean }) {
  const [v, setV] = useState("");
  const add = () => {
    const tag = v.trim();
    if (tag && !tags.includes(tag)) onChange([...tags, tag]);
    setV("");
  };
  return (
    <Field label={t("Tags")} htmlFor="post-tags">
      <div className="stack">
        <div className="row">
          {tags.map((tag) => (
            <span key={tag} className="chip">
              {isolate(tag)}
              <button type="button" className="chip-x" disabled={locked} aria-label={t("Remove the tag {tag}", { tag: isolate(tag) })} onClick={() => onChange(tags.filter((x) => x !== tag))}>
                ×
              </button>
            </span>
          ))}
        </div>
        <input id="post-tags" className="input" list="post-tags-known" value={v} disabled={locked} placeholder={t("Add a tag and press Enter")} onChange={(e) => setV(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); add(); } }} onBlur={add} />
        <datalist id="post-tags-known">
          {known.map((k) => <option key={k} value={k} />)}
        </datalist>
      </div>
    </Field>
  );
}

export function ComposerRoute() {
  const { id: param } = useParams();
  const postId = param && param !== "new" ? param : null;
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const client = useQueryClient();
  const accounts = useAccounts();
  const brands = useBrands();
  const library = useLibrary();
  const post = useQuery({ queryKey: ["social", "post", postId], queryFn: () => record("SocialPost", postId!), enabled: !!postId });
  const tagsQ = useQuery({ queryKey: ["social", "post-tags", postId], queryFn: () => api.get<{ tags: string[] }>(docPath("SocialPost", postId!, "tags")).then((r) => r.tags).catch(() => [] as string[]), enabled: !!postId });
  const knownTags = useQuery({ queryKey: ["social", "tags"], queryFn: () => api.get<{ tags: string[] | { tag: string }[] }>("/api/tags/SocialPost").then((r) => r.tags.map((x) => (typeof x === "string" ? x : x.tag))).catch(() => [] as string[]) });
  const canWrite = useAllowed("SocialPost", "write");
  const canCreate = useAllowed("SocialPost", "create");
  const canDelete = useAllowed("SocialPost", "delete");

  const [draft, setDraft] = useState<Draft | null>(null);
  const [baseline, setBaseline] = useState("");
  const [mode, setMode] = useState<Mode>("draft");
  const [tab, setTab] = useState("original");
  const [refused, setRefused] = useState("");
  const [headlineError, setHeadlineError] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [asTemplate, setAsTemplate] = useState(false);
  const loaded = useRef<string | null>(null);

  // A post is read once per version (its tags come apart from it and are waited for); a new one starts empty, with what the
  // address names (an account, a time).
  useEffect(() => {
    if (postId) {
      const row = post.data;
      if (!row || tagsQ.data === undefined || loaded.current === `${row.id}@${row.version}`) return;
      loaded.current = `${row.id}@${row.version}`;
      const d = { ...fromRecord(row), tags: tagsQ.data };
      setDraft(d);
      setMode(modeOf(d));
      setBaseline(snap(d, text(row.brand) || null));
    } else if (accounts.data && !loaded.current) {
      loaded.current = "new";
      const d = emptyDraft();
      const account = params.get("account");
      if (account && accounts.data.some((a) => a.id === account)) d.targets.push({ account, kind: "", settings: {} });
      const at = params.get("at");
      if (at) d.at = at;
      setDraft(d);
      setMode(d.at ? "time" : "draft");
      setBaseline(snap(d, null));
      // Started from a template or a set of the library (/library): its content fills the draft, which is then unsaved.
      const tpl = params.get("template");
      const set_ = params.get("set");
      if (tpl) void record("SocialTemplate", tpl).then((r) => setDraft({ ...fromRecord(r), headline: text(r.headline) || text(r.template_name), at: d.at }));
      else if (set_) void record("SocialSet", set_).then((r) => setDraft({ ...fromRecord(r), headline: text(r.set_name), at: d.at }));
    }
  }, [postId, post.data, tagsQ.data, accounts.data, params]);

  const all = accounts.data ?? [];
  const chosen = draft?.targets.map((x) => x.account) ?? [];
  const brand = text(all.find((a) => chosen.includes(a.id))?.brand) || text(post.data?.brand) || null;
  const rulesQ = useQueries({
    queries: chosen.map((a) => ({ queryKey: ["social", "rules", a], queryFn: () => run<{ rules: Rules }>("SocialAccount", a, "rules").then((r) => ({ ...NO_RULES, ...r.rules })), staleTime: 60_000, retry: false })),
  });
  const rules: Record<string, Rules> = {};
  chosen.forEach((a, i) => { rules[a] = rulesQ[i]?.data ?? NO_RULES; });
  const status = text(post.data?.status) || "Draft";
  const locked = (postId ? !canWrite : !canCreate) || LOCKED.includes(status);
  const dirty = !!draft && snap(draft, brand) !== baseline;
  const { allowLeave, dialog: guard } = useUnsavedGuard(dirty);

  const lib = library.data;
  // What the server counts and checks for the post as it stands now, saved or not (nothing is written): asked after a pause, the
  // answer to a newer body replacing an older (the query is keyed by the body), the last one kept and dimmed meanwhile.
  const live = draft && chosen.length ? JSON.stringify({ ...toRecord(draft, brand, rules), want_checks: true }) : "";
  const body = useDebounced(live, 400);
  const calc = useQuery<ServerCopy[]>({
    queryKey: ["social", "calc", body],
    queryFn: ({ signal }) => api.post<{ checks: { copies: ServerCopy[] } }>(docPath("SocialPost", "_calc"), JSON.parse(body), { signal }).then((r) => r.checks.copies),
    enabled: !!body,
    placeholderData: keepPreviousData,
    retry: false,
    staleTime: Infinity,
  });
  const dimmed = live !== body || calc.isFetching;
  const copies = calc.data ?? [];

  const set = (patch: Partial<Draft> | ((d: Draft) => Draft)) => setDraft((d) => (d ? (typeof patch === "function" ? patch(d) : { ...d, ...patch }) : d));
  const toggle = (account: string) => {
    set((d) => (d.targets.some((x) => x.account === account) ? removeAccount(d, account) : { ...d, targets: [...d.targets, { account, kind: "", settings: {} }] }));
    if (tab === `acc-${account}`) setTab("original");
  };

  /** Saves the post (a new one is made, a known one changed as it was read); returns its record, or null with the server's words shown. */
  const save = async (): Promise<Row | null> => {
    if (!draft) return null;
    setRefused("");
    setHeadlineError("");
    const noHeadline = !draft.headline.trim();
    if (noHeadline) setHeadlineError(t("Give the post a headline, what the team calls it"));
    if (!chosen.length) setRefused(t("Choose at least one account"));
    if (noHeadline || !chosen.length) return null;
    const empty = chosen.find((a) => draft.versions[a] && !draft.versions[a].items[0].text.trim());
    if (empty) {
      setRefused(t("{name} has a version of its own with no words: write some, or go back to the original", { name: isolate(text(all.find((a) => a.id === empty)?.account_name)) }));
      return null;
    }
    setSaving(true);
    try {
      const body = toRecord(draft, brand, rules);
      const row = postId ? await change("SocialPost", postId, body, post.data?.version) : await make("SocialPost", body);
      if (JSON.stringify([...draft.tags].sort()) !== JSON.stringify([...(tagsQ.data ?? [])].sort()) || !postId) {
        if (draft.tags.length || tagsQ.data?.length) await api.put(docPath("SocialPost", row.id, "tags"), { tags: draft.tags });
      }
      setBaseline(snap(draft, brand));
      await client.invalidateQueries({ queryKey: ["social"] });
      return row;
    } catch (e) {
      setRefused(refusal(e));
      return null;
    } finally {
      setSaving(false);
    }
  };
  const onSave = async () => {
    setBusy("save");
    const row = await save();
    if (row) {
      toast(t("Saved"));
      if (!postId) {
        allowLeave();
        navigate(`/compose/${row.id}`, { replace: true });
      }
    }
    setBusy(null);
  };
  /** Saves first when there is something to save, then runs the action on the post. */
  const act = async (name: string, params: Record<string, unknown> = {}, done?: (out: Record<string, unknown>) => string | void) => {
    setBusy(name);
    try {
      let row: Row | undefined = postId ? post.data : undefined;
      if (dirty || !row) {
        const saved = await save();
        if (!saved) return;
        row = saved;
      }
      const out = await run<Record<string, unknown>>("SocialPost", row.id, name, params);
      const said = done?.(out);
      if (said) toast(said);
      await client.invalidateQueries({ queryKey: ["social"] });
      if (name === "duplicate") {
        allowLeave();
        navigate(`/compose/${out.id}`);
        return;
      }
      if (!postId) {
        allowLeave();
        navigate(`/compose/${row.id}`, { replace: true });
      }
    } catch (e) {
      setRefused(refusal(e));
    } finally {
      setBusy(null);
    }
  };
  const del = async () => {
    if (!postId || !(await confirmDialog({ title: t("Delete this post?"), message: t("It is removed for good."), confirmLabel: t("Delete"), danger: true }))) return;
    try {
      await remove("SocialPost", postId);
      allowLeave();
      navigate("/");
    } catch (e) {
      setRefused(refusal(e));
    }
  };

  const q = firstOf(...[accounts, brands, library, ...(postId ? [post] : [])]);
  if (q.isPending || q.isError || !draft)
    return (
      <PageStack>
        <Nav />
        <Await q={q} skeleton={<FormSkeleton fields={6} />}>{() => <FormSkeleton fields={6} />}</Await>
      </PageStack>
    );

  const sigs = lib?.signatures ?? [];
  // (an account the person may not read is drawn by its id, not left out)
  const accountRow = (a: string): Row => all.find((x) => x.id === a) ?? { id: a, account_name: t("An account you cannot see") };
  const versions = chosen.filter((a) => draft.versions[a]);
  const loose = chosen.filter((a) => !draft.versions[a]);
  const tabs = [{ id: "original", label: t("Original") }, ...versions.map((a) => ({ id: `acc-${a}`, label: isolate(text(accountRow(a).account_name)) }))];
  const here = tab.startsWith("acc-") && draft.versions[tab.slice(4)] ? tab.slice(4) : null;
  const content = here ? draft.versions[here] : draft.original;
  const hereRules = here ? rules[here] : null;
  const hereEff = here ? effective(hereRules!, draft.targets.find((x) => x.account === here)?.kind || null) : null;
  const follows = chosen.map((a) => effective(rules[a], draft.targets.find((x) => x.account === a)?.kind || null).follow.as);
  const common = hereEff ? hereEff.follow.as : follows.length && follows.every((f) => f === follows[0]) ? follows[0] : null;
  const snippets: Insertable[] = (lib?.snippets ?? []).filter((s) => !text(s.brand) || text(s.brand) === brand).map((s) => ({ label: text(s.snippet_name), text: text(s.text) }));
  const names = [...new Set((lib?.variables ?? []).map((v) => text(v.variable_name)).filter(Boolean))];
  const wantsComment = chosen.some((a) => effective(rules[a], draft.targets.find((x) => x.account === a)?.kind || null).firstComment);
  const waitNote = text(post.data?.waiting_for);

  const applySet = async (setId: string) => {
    const s = await record("SocialSet", setId);
    const next = fromRecord({ ...s, headline: draft.headline || text(s.set_name), text: text(s.text) || draft.original.items[0].text } as Row);
    set({ ...next, headline: draft.headline || text(s.set_name), original: text(s.text) ? next.original : draft.original, at: draft.at, tags: draft.tags });
  };
  const applyTemplate = async (tid: string) => {
    const tpl = await record("SocialTemplate", tid);
    const next = fromRecord(tpl);
    set({ ...next, headline: draft.headline || text(tpl.headline) || text(tpl.template_name), at: draft.at, tags: draft.tags, repeatEvery: draft.repeatEvery, repeatUnit: draft.repeatUnit, repeatUntil: draft.repeatUntil, repeatCount: draft.repeatCount });
  };
  const when = (iso: unknown) => formatMoment(text(iso), { dateStyle: "medium", timeStyle: "short" });
  const editable = !locked;

  const buttons = (
    <>
      {postId && <ButtonLink to={`/form/SocialPost/${postId}`}>{t("Open the form")}</ButtonLink>}
      {editable && (
        <Button loading={busy === "save"} disabled={!!busy || saving} onClick={() => void onSave()}>
          {status === "Approved" ? t("Save changes") : t("Save draft")}
        </Button>
      )}
      {editable && ["Draft", "Rejected"].includes(status) && (
        <Button variant={mode === "draft" || mode === "time" ? "primary" : "default"} loading={busy === "send_for_review"} disabled={!!busy || saving} onClick={() => void act("send_for_review", {}, (o) => (o.status === "Approved" || o.status === "Sending" ? t("Approved: {status}", { status: statusWord(String(o.status)) }) : t("Sent for review")))}>
          {t("Send for review")}
        </Button>
      )}
      {editable && mode === "queue" && (
        <Button variant="primary" loading={busy === "add_to_queue"} disabled={!!busy || saving} onClick={() => void act("add_to_queue", {}, (o) => t("In the queue for {at}", { at: when(o.scheduled_at) }))}>
          {t("Add to queue")}
        </Button>
      )}
      {editable && mode === "now" && status !== "In Review" && (
        <Button variant="primary" loading={busy === "publish_now"} disabled={!!busy || saving} onClick={() => void act("publish_now", {}, (o) => (o.status === "Sending" ? t("Sending now") : t("Waiting for approval before it goes: {status}", { status: statusWord(String(o.status)) })))}>
          {t("Publish now")}
        </Button>
      )}
      {postId && (
        <Menu label={t("More")} ariaLabel={t("More actions")}>
          {(close) => (
            <>
              <MenuItem onSelect={() => { close(); void act("duplicate", {}); }}>{t("Duplicate")}</MenuItem>
              <MenuItem onSelect={() => { close(); setAsTemplate(true); }}>{t("Save as template")}</MenuItem>
              {["In Review", "Approved"].includes(status) && <MenuItem onSelect={() => { close(); void (async () => { if (await confirmDialog({ title: t("Back to a draft?"), message: t("It leaves the calendar and needs approval again."), confirmLabel: t("Back to draft") })) void act("unschedule"); })(); }}>{t("Back to draft")}</MenuItem>}
              {status === "Failed" && <MenuItem onSelect={() => { close(); void act("retry", {}, () => t("Sending again")); }}>{t("Retry")}</MenuItem>}
              {draft.repeatEvery && <MenuItem onSelect={() => { close(); void act("stop_repeat", {}, () => t("It repeats no more")); }}>{t("Stop repeating")}</MenuItem>}
              {canDelete && <MenuItem danger onSelect={() => { close(); void del(); }}>{t("Delete")}</MenuItem>}
            </>
          )}
        </Menu>
      )}
    </>
  );

  return (
    <PageStack>
      <Nav />
      {guard}
      <PageHead
        title={postId ? t("Edit post") : t("New post")}
        description={postId ? undefined : t("Write once, then give any account its own version.")}
        actions={buttons}
      />
      {refused && <Alert tone="error">{isolate(refused)}</Alert>}
      {postId && (
        <div className="row">
          <StatusBadge value={status} tones={POST_TONES} label={statusWord(status)} />
          {text(post.data?.scheduled_at) && <span className="muted">{when(post.data?.scheduled_at)}</span>}
          {waitNote && <Alert tone="warning">{t("Waiting: {why}", { why: isolate(waitNote) })}</Alert>}
          {status === "Failed" && <Alert tone="error">{isolate(text(post.data?.failure))}</Alert>}
          {draft.repeatEvery && post.data?.repeat_index != null && <Badge>{t("Repeat {n}", { n: String(post.data.repeat_index) })}</Badge>}
          {text(post.data?.repeat_note) && <span className="muted">{isolate(text(post.data?.repeat_note))}</span>}
          {text(post.data?.repeat_of) && <Link to={`/compose/${text(post.data?.repeat_of)}`}>{t("The first of the repeats")}</Link>}
        </div>
      )}
      {locked && <Alert tone="info">{LOCKED.includes(status) ? t("A post stays as it went out. Duplicate it to change it.") : t("You may only read this post.")}</Alert>}
      {status === "Approved" && editable && <Alert tone="info">{t("A change to an approved post needs approval again.")}</Alert>}
      <PageSplit
        main={
          <div className="stack">
            <div className="card card-body stack">
              <Field label={t("Headline")} required htmlFor="post-headline" error={headlineError || undefined} hint={t("What the team calls it.")}>
                <input id="post-headline" className="input" disabled={locked} value={draft.headline} onChange={(e) => set({ headline: e.target.value })} />
              </Field>
              <Pick accounts={all} brands={brands.data ?? []} chosen={chosen} onToggle={toggle} locked={locked} />
              {!locked && (
                <div className="row">
                  {(lib?.sets ?? []).length > 0 && (
                    <select className="select" aria-label={t("Start from a set")} value="" onChange={(e) => e.target.value && void applySet(e.target.value)}>
                      <option value="">{t("Start from a set")}</option>
                      {lib!.sets.filter((s) => !brand || text(s.brand) === brand).map((s) => <option key={s.id} value={s.id}>{isolate(text(s.set_name))}</option>)}
                    </select>
                  )}
                  {(lib?.templates ?? []).length > 0 && (
                    <select className="select" aria-label={t("Use a template")} value="" onChange={(e) => e.target.value && void applyTemplate(e.target.value)}>
                      <option value="">{t("Use a template")}</option>
                      {lib!.templates.filter((s) => !brand || !text(s.brand) || text(s.brand) === brand).map((s) => <option key={s.id} value={s.id}>{isolate(text(s.template_name))}</option>)}
                    </select>
                  )}
                </div>
              )}
            </div>

            {chosen.length > 0 && (
              <div className="card card-body stack">
                <div className="row">
                  <div className="spacer"><Tabs id="versions" tabs={tabs} value={here ? tab : "original"} onChange={setTab} /></div>
                  {!locked && loose.length > 0 && (
                    <Menu label={t("Customise for…")} ariaLabel={t("Customise for an account")}>
                      {(close) => loose.map((a) => <MenuItem key={a} onSelect={() => { close(); set((d) => customise(d, a)); setTab(`acc-${a}`); }}>{isolate(text(accountRow(a).account_name))}</MenuItem>)}
                    </Menu>
                  )}
                </div>
                <TabPanel id="versions" tab={here ? tab : "original"}>
                  <div className="stack">
                    {here ? (
                      <div className="row">
                        <Face name={text(accountRow(here).account_name)} picture={text(accountRow(here).picture)} />
                        <span className="spacer">{t("The words of {name} alone: nothing of the original is mixed in.", { name: nameOf(accountRow(here)) })}</span>
                        {!locked && <Button size="sm" onClick={() => { set((d) => dropVersion(d, here)); setTab("original"); }}>{t("Back to the original")}</Button>}
                      </div>
                    ) : (
                      versions.length < chosen.length && <p className="muted">{t("Every account without a version of its own gets this.")}</p>
                    )}
                    <Items
                      key={here ?? "original"}
                      idPrefix={here ?? "original"}
                      content={content}
                      onChange={(c) => set((d) => (here ? { ...d, versions: { ...d.versions, [here]: c } } : { ...d, original: c }))}
                      followLabel={common && common !== "none" ? followWord(common) : ""}
                      followNone={common === "none"}
                      counts={here ? (copies.find((c) => c.account === here)?.items ?? null) : null}
                      media={hereEff?.media ?? null}
                      snippets={snippets}
                      variables={names}
                      locked={locked}
                      brand={brand}
                      mentionAccount={here ? (rules[here].mentions ? here : null) : (chosen.find((a) => rules[a].mentions) ?? null)}
                    />
                    {wantsComment && (
                      <Field label={t("First comment")} htmlFor="post-first-comment" hint={t("Added under the post right after it goes out, where the network takes one.")}>
                        <textarea id="post-first-comment" className="textarea" rows={2} disabled={locked} value={draft.firstComment} onChange={(e) => set({ firstComment: e.target.value })} />
                      </Field>
                    )}
                  </div>
                </TabPanel>
              </div>
            )}

            {chosen.map((a) => {
              const r = rules[a];
              const target = draft.targets.find((x) => x.account === a)!;
              if (!r.kinds.length && !r.settings.length) return null;
              return (
                <details key={a} className="card card-body" open>
                  <summary>{t("What {name} asks", { name: isolate(text(accountRow(a).account_name)) })}</summary>
                  <AccountOptions account={a} rules={r} kind={target.kind} answers={target.settings} locked={locked}
                    onKind={(kind) => set((d) => ({ ...d, targets: d.targets.map((x) => (x.account === a ? { ...x, kind } : x)) }))}
                    onAnswers={(settings) => set((d) => ({ ...d, targets: d.targets.map((x) => (x.account === a ? { ...x, settings } : x)) }))} />
                </details>
              );
            })}

            <div className="card card-body stack">
              <label className="row">
                <input type="checkbox" disabled={locked} checked={draft.addSignature} onChange={(e) => set({ addSignature: e.target.checked })} />
                {t("Add the signature")}
              </label>
              {draft.addSignature && (
                <Field label={t("Which signature")} htmlFor="post-signature" hint={t("Empty: the account's own, else the brand's, else the workspace's.")}>
                  <select id="post-signature" className="select" disabled={locked} value={draft.signature} onChange={(e) => set({ signature: e.target.value })}>
                    <option value="">{t("The default")}</option>
                    {sigs.filter((s) => !text(s.brand) || text(s.brand) === brand).map((s) => <option key={s.id} value={s.id}>{isolate(text(s.signature_name))}</option>)}
                  </select>
                </Field>
              )}
              <Tags tags={draft.tags} known={knownTags.data ?? []} locked={locked} onChange={(tags) => set({ tags })} />
            </div>

            <When draft={draft} mode={mode} locked={locked} onMode={(m) => { setMode(m); if (m !== "time") set({ at: "" }); }} onChange={setDraft as (d: Draft) => void}
              note={status === "Approved" || status === "In Review" ? t("A change of time keeps the approval; a change of words asks again.") : undefined} />
          </div>
        }
        aside={
          <div className="stack">
            <h2 className="section-title">{t("Preview")}</h2>
            {chosen.length === 0 && <p className="muted">{t("Choose an account to see how the post looks there.")}</p>}
            {!locked && (
              <Writing postId={postId} brandId={brand} draft={draft} accounts={all} dirty={dirty} onSave={() => void onSave()}
                take={{
                  words: (words, account) => set((d) => { const x = account ? customise(d, account) : d; return account ? { ...x, versions: { ...x.versions, [account]: { items: x.versions[account].items.map((it, n) => (n ? it : { ...it, text: words })) } } } : { ...x, original: { items: x.original.items.map((it, n) => (n ? it : { ...it, text: words })) } }; }),
                  versions: (list: Adapted[]) => set((d) => list.reduce((acc, v) => { const x = customise(acc, v.account); return { ...x, versions: { ...x.versions, [v.account]: { items: x.versions[v.account].items.map((it, n) => (n ? it : { ...it, text: v.text })) } } }; }, d)),
                  thread: (parts) => set((d) => ({ ...d, original: { items: parts.map((p, n) => (n === 0 ? { ...d.original.items[0], text: p } : { ...blankItem(), text: p })) } })),
                  hashtags: (tags, account) => set((d) => { const x = account ? customise(d, account) : d; const c = contentFor(x, account); const first = { ...c.items[0], text: `${c.items[0].text.trimEnd()}\n\n${tags.join(" ")}`.trim() }; const items = [first, ...c.items.slice(1)]; return account ? { ...x, versions: { ...x.versions, [account]: { items } } } : { ...x, original: { items } }; }),
                  alt: (key, alt) => set((d) => { const f = (c: { items: typeof d.original.items }) => ({ items: c.items.map((it) => ({ ...it, media: it.media.map((m) => (m.key === key ? { ...m, alt } : m)) })) }); return { ...d, original: f(d.original), versions: Object.fromEntries(Object.entries(d.versions).map(([a, c]) => [a, f(c)])) }; }),
                  picture: async (assetId) => { const a = await record("SocialMediaAsset", assetId); set((d) => ({ ...d, original: { items: d.original.items.map((it, n) => (n ? it : { ...it, media: [...it.media, mediumOfAsset(a)] })) } })); },
                }} />
            )}
            {calc.isError && <Alert tone="error">{refusal(calc.error)}</Alert>}
            {chosen.map((a) => (
              <Preview key={a} account={accountRow(a)} copy={copies.find((c) => c.account === a)} media={contentFor(draft, a).items.map((i) => i.media)} dimmed={dimmed} />
            ))}
            {postId && status === "Published" && <section className="card card-body stack"><h2 className="section-title">{t("What it did")}</h2><PostNumbers post={postId} accounts={all} /></section>}
            {postId && post.data && <ReviewPanel post={post.data} onChanged={() => void client.invalidateQueries({ queryKey: ["social"] })} />}
          </div>
        }
      />
      {asTemplate && post.data && <SaveAsTemplate post={post.data} onClose={() => setAsTemplate(false)} />}
    </PageStack>
  );
}
